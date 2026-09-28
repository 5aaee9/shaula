use super::*;
use crate::http_state::MigrationClassification as Class;

#[tokio::test]
async fn legacy_verified_destroy_keeps_empty_state_completion_proof() -> TestResult {
    let f = Fixture::new().await?;
    let id = f.claim.generation_id.to_string();
    f.store
        .connection()
        .execute(sql(
            "DELETE FROM generation_http_state WHERE generation_id = ?",
            vec![id.clone().into()],
        ))
        .await?;
    f.store
        .generation_set_result(
            &id,
            r#"{"state_lineage":"original","state_serial":1}"#,
            "fixture",
            3,
        )
        .await?;
    f.store.connection().execute(sql("UPDATE runner_generations SET state = 'Destroying', resources_destroyed_at = 4 WHERE id = ?", vec![id.clone().into()])).await?;
    let generation = f
        .backend
        .legacy_generations()
        .await?
        .pop()
        .ok_or("legacy absent")?;
    f.backend
        .import_legacy(
            &generation,
            &generation.commitment()?,
            Some(state(2, "original", false)?),
            Class::Imported,
            Some(b"original input".to_vec()),
            Some("http/generation"),
        )
        .await?;
    let record = f
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("worker absent")?;
    let admissions = crate::http_state::WorkerAdmissions::new(4, 1)?;
    f.backend
        .recover_worker(
            &record,
            shaula_core::worker::FenceOutcome::Fenced,
            &admissions,
            false,
        )
        .await?;
    assert!(f.backend.complete_generation(&id, 5).await?);
    assert_eq!(
        f.store
            .generation_get(&id)
            .await?
            .ok_or("generation absent")?
            .state,
        "Destroyed"
    );
    Ok(())
}

#[tokio::test]
async fn legacy_classification_blocks_activation_and_rejects_stale_database() -> TestResult {
    let f = Fixture::new().await?;
    assert!(!f.backend.activated().await?);
    assert!(f.backend.activate().await.is_err());
    let generation = f
        .backend
        .legacy_generations()
        .await?
        .pop()
        .ok_or("legacy absent")?;
    let commitment = generation.commitment()?;
    f.store
        .connection()
        .execute(sql(
            "UPDATE runner_generations SET updated_at = updated_at + 1 WHERE id = ?",
            vec![generation.id.clone().into()],
        ))
        .await?;
    assert!(f
        .backend
        .import_legacy(
            &generation,
            &commitment,
            None,
            Class::Quarantined,
            None,
            None
        )
        .await
        .is_err());
    let current = f
        .backend
        .legacy_generations()
        .await?
        .pop()
        .ok_or("legacy absent")?;
    let commitment = current.commitment()?;
    f.backend
        .import_legacy(&current, &commitment, None, Class::Quarantined, None, None)
        .await?;
    f.backend
        .import_legacy(&current, &commitment, None, Class::Quarantined, None, None)
        .await?;
    f.backend.activate().await?;
    assert!(f.backend.activated().await?);
    assert_eq!(
        f.store
            .generation_get(&current.id)
            .await?
            .ok_or("generation absent")?
            .state,
        "Quarantined"
    );
    Ok(())
}

#[tokio::test]
async fn legacy_import_preserves_exact_bytes_and_replay_content_identity() -> TestResult {
    let f = Fixture::new().await?;
    // Model a true pre-HTTP generation, with no pre-existing backend record.
    f.store
        .connection()
        .execute(sql(
            "DELETE FROM generation_http_state WHERE generation_id = ?",
            vec![f.claim.generation_id.to_string().into()],
        ))
        .await?;
    f.store
        .generation_set_result(
            &f.claim.generation_id.to_string(),
            r#"{"state_lineage":"legacy-lineage","state_serial":7}"#,
            "fixture",
            3,
        )
        .await?;
    let generation = f
        .backend
        .legacy_generations()
        .await?
        .pop()
        .ok_or("legacy absent")?;
    let document = state(7, "legacy-lineage", true)?;
    let original = document.bytes().to_vec();
    let commitment = generation.commitment()?;
    f.backend
        .import_legacy(
            &generation,
            &commitment,
            Some(document),
            Class::Imported,
            Some(b"original input".to_vec()),
            Some("http/generation"),
        )
        .await?;
    let row = f.store.connection().query_one(sql("SELECT state_bytes, revoked, create_started FROM generation_http_state WHERE generation_id = ?", vec![generation.id.clone().into()])).await?.ok_or("state absent")?;
    assert_eq!(row.try_get::<Vec<u8>>("", "state_bytes")?, original);
    assert!(row.try_get::<bool>("", "revoked")?);
    assert!(row.try_get::<bool>("", "create_started")?);
    f.backend
        .import_legacy(
            &generation,
            &commitment,
            Some(StateDocument::parse(original)?),
            Class::Imported,
            Some(b"original input".to_vec()),
            Some("http/generation"),
        )
        .await?;
    assert!(f
        .backend
        .import_legacy(
            &generation,
            &commitment,
            Some(state(8, "legacy-lineage", true)?),
            Class::Imported,
            Some(b"original input".to_vec()),
            Some("http/generation")
        )
        .await
        .is_err());
    f.backend.activate().await?;
    Ok(())
}

#[tokio::test]
async fn legacy_import_rejects_foreign_rolled_back_or_unproven_state() -> TestResult {
    for result in [
        None,
        Some(r#"{"state_lineage":"foreign","state_serial":1}"#),
        Some(r#"{"state_lineage":"legacy-lineage","state_serial":8}"#),
    ] {
        let f = Fixture::new().await?;
        let id = f.claim.generation_id.to_string();
        f.store
            .connection()
            .execute(sql(
                "DELETE FROM generation_http_state WHERE generation_id = ?",
                vec![id.clone().into()],
            ))
            .await?;
        if let Some(result) = result {
            f.store
                .generation_set_result(&id, result, "fixture", 3)
                .await?;
        }
        let generation = f
            .backend
            .legacy_generations()
            .await?
            .pop()
            .ok_or("legacy absent")?;
        assert!(f
            .backend
            .import_legacy(
                &generation,
                &generation.commitment()?,
                Some(state(7, "legacy-lineage", true)?),
                Class::Imported,
                Some(b"original input".to_vec()),
                Some("http/generation")
            )
            .await
            .is_err());
        assert!(
            !f.backend
                .import_receipt(&id, &generation.commitment()?)
                .await?
        );
        assert_eq!(f.backend.legacy_generations().await?.len(), 1);
    }
    Ok(())
}
