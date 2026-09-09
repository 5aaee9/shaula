use super::*;
use shaula_core::{lifecycle::GenerationState, ports::StateLineage, registry::GenerationRecord};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn fixture() -> Result<(tempfile::TempDir, Store, PlanProvenance), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let store = Store::open(&directory.path().join("bootstrap.db")).await?;
    store.migrate().await?;
    store.connection().execute_unprepared(
        "INSERT INTO fleets (key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at)
         VALUES ('fleet','inc',1,0,1,0,'Ready',0,1,1)"
    ).await?;
    store
        .generation_insert(GenerationRecord {
            id: "generation".into(),
            fleet_key: "fleet".into(),
            runner_name: "runner".into(),
            generation_name: "generation".into(),
            fleet_revision: 1,
            template_profile_key: "profile".into(),
            template_revision: 1,
            template_artifact_digest: "sha256:artifact".into(),
            attestation_id: "attestation".into(),
            inputs_digest: "sha256:input".into(),
            state: GenerationState::Creating,
            github_runner_id: None,
            workspace_path: "workspace".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await?;
    store
        .generation_advance("generation", GenerationState::Creating, None, 1)
        .await?;
    let provenance = PlanProvenance {
        intent: PlanIntent::Create,
        saved_plan_digest: "sha256:plan".into(),
        engine_kind: "terraform".into(),
        engine_version: "1.9.8".into(),
        engine_binary_digest: "sha256:engine".into(),
        artifact_digest: "sha256:artifact".into(),
        template_material_digest: "sha256:material".into(),
        protected_input_digest: "sha256:input".into(),
        state_lineage: StateLineage::Empty,
        generation_id: "generation".into(),
        attempt_id: "attempt".into(),
    };
    store
        .operation_insert(shaula_core::registry::OperationInsert {
            id: provenance.attempt_id.clone(),
            generation_id: provenance.generation_id.clone(),
            kind: "Create".into(),
            state: "ApplyStarting".into(),
            provenance_json: Some(serde_json::to_string(&provenance)?),
            saved_plan_path: Some("tfplan".into()),
            saved_plan_digest: Some(provenance.saved_plan_digest.clone()),
            now: 1,
        })
        .await?;
    Ok((directory, store, provenance))
}

#[tokio::test]
async fn bootstrap_is_single_use_and_remains_open_for_recovery() -> TestResult {
    let (_directory, store, provenance) = fixture().await?;
    let (first, second) = tokio::join!(
        store.operation_bootstrap_starting(&provenance, 2),
        store.operation_bootstrap_starting(&provenance, 2)
    );
    assert_ne!(
        first?, second?,
        "only one concurrent continuation may acquire authorization"
    );
    let open = store.operations_open_for_generation("generation").await?;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].state, "BootstrapStarting");
    assert!(!store.operation_bootstrap_starting(&provenance, 3).await?);
    Ok(())
}

#[tokio::test]
async fn bootstrap_refuses_changed_authority_and_provenance() -> TestResult {
    for update in [
        "UPDATE fleets SET desired_revision=2",
        "UPDATE fleets SET deletion_marker=1",
        "UPDATE fleets SET tombstone=1",
        "UPDATE runner_generations SET state='CleanupRequired'",
        "UPDATE runner_operations SET provenance_json='{}'",
    ] {
        let (_directory, store, provenance) = fixture().await?;
        store.connection().execute_unprepared(update).await?;
        assert!(
            !store.operation_bootstrap_starting(&provenance, 2).await?,
            "{update}"
        );
    }
    let (_directory, store, mut provenance) = fixture().await?;
    provenance.intent = PlanIntent::Destroy;
    assert!(!store.operation_bootstrap_starting(&provenance, 2).await?);
    Ok(())
}
