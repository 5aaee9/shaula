use super::*;
use crate::http_state::MigrationClassification as Class;

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
