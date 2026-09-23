//! Commit-time replacement fences must survive an earlier empty admission read.
use super::forgejo_pool_support::{Fixture, TestResult};
use sea_orm::ConnectionTrait;
use shaula_core::{
    lifecycle::GenerationState as G,
    registry::{
        ChangeView, ControlPlaneStore, LifecycleStore, MutationError, MutationFacts,
        OperationInsert,
    },
};

async fn replacement(fixture: &Fixture) -> TestResult<MutationFacts> {
    fixture.store.store().connection().execute_unprepared(
        "INSERT INTO github_auth_profiles
        (key,incarnation,desired_revision,active_revision,status,deletion_requested,created_at,updated_at)
        VALUES ('forgejo','auth-inc',1,1,'Active',0,1,1)",
    ).await?;
    let previous = fixture
        .store
        .fleet_revision_latest("fleet")
        .await?
        .ok_or("missing fleet")?;
    let mut spec = fixture.spec.clone();
    spec.template_inputs.insert("size".into(), "large".into());
    Ok(MutationFacts {
        resource_kind: "fleet",
        resource_key: "fleet".into(),
        incarnation: "inc".into(),
        revision: 2,
        spec_json: serde_json::to_string(&spec)?,
        template: Some((
            "profile".into(),
            1,
            previous
                .template_artifact_digest
                .ok_or("missing artifact")?,
            "attestation".into(),
        )),
        template_pool: Vec::new(),
        template_pool_ref: None,
        auth_desired: Some(previous.auth_desired),
        inputs_digest: "new-inputs".into(),
        actor: "test".into(),
        now: 10,
        change: ChangeView {
            id: "replace".into(),
            resource_kind: "fleet".into(),
            resource_key: "fleet".into(),
            revision: 2,
            kind: "Replace".into(),
            state: "Pending".into(),
            reason: None,
        },
        outbox_topic: "fleet.reconcile".into(),
        outbox_payload: "{}".into(),
        idempotency: None,
    })
}

#[tokio::test]
async fn inputs_only_commit_rechecks_a_create_after_the_admission_read() -> TestResult {
    let fixture = Fixture::new(0, 2).await?;
    let facts = replacement(&fixture).await?;
    assert_eq!(fixture.store.generations_occupancy("fleet").await?, 0);
    // Deterministic race schedule: Create lands after admission, before commit.
    fixture.seed(G::CreatePending).await?;
    assert!(matches!(
        fixture.store.commit_fleet_mutation(facts).await?,
        Err(MutationError::RetirementBlocked { .. })
    ));
    assert_eq!(
        fixture
            .store
            .fleet_get("fleet")
            .await?
            .ok_or("missing head")?
            .desired_revision,
        1
    );
    assert!(fixture.store.fleet_change_get("replace").await?.is_none());
    Ok(())
}

#[tokio::test]
async fn inputs_only_commit_waits_for_operations_even_after_destroyed() -> TestResult {
    let fixture = Fixture::new(0, 2).await?;
    let facts = replacement(&fixture).await?;
    fixture.seed(G::Idle).await?;
    fixture
        .store
        .operation_insert(OperationInsert {
            id: "late-effect".into(),
            generation_id: "generation".into(),
            kind: "Create".into(),
            state: "Blocked".into(),
            provenance_json: None,
            saved_plan_path: None,
            saved_plan_digest: None,
            now: 1,
        })
        .await?;
    for state in [G::Retiring, G::DestroyPending, G::Destroying, G::Destroyed] {
        fixture
            .store
            .generation_advance("generation", state, 2)
            .await?;
    }
    assert_eq!(fixture.store.generations_occupancy("fleet").await?, 0);
    assert!(matches!(
        fixture.store.commit_fleet_mutation(facts.clone()).await?,
        Err(MutationError::RetirementBlocked { .. })
    ));
    fixture
        .store
        .operation_update_state("late-effect", "Completed", 3)
        .await?;
    assert!(fixture.store.commit_fleet_mutation(facts).await?.is_ok());
    Ok(())
}

#[tokio::test]
async fn equal_inputs_do_not_block_capacity_updates_over_live_generations() -> TestResult {
    let fixture = Fixture::new(0, 2).await?;
    let mut facts = replacement(&fixture).await?;
    let mut spec = fixture.spec.clone();
    spec.capacity.max_runners = 3;
    facts.spec_json = serde_json::to_string(&spec)?;
    // The comparison is semantic inputs, not an opaque digest supplied by a caller.
    fixture.seed(G::Idle).await?;
    assert!(fixture.store.commit_fleet_mutation(facts).await?.is_ok());
    Ok(())
}
