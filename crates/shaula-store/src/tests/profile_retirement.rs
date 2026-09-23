//! Exercise retirement through public commits/scans over real SQLite.
use super::{forgejo_pool_support::TestResult, profile_retirement_support::*};
use crate::{registry_impl::SqliteControlPlane, Store};
use sea_orm::ConnectionTrait;
use shaula_core::{
    lifecycle::GenerationState as G,
    registry::{ControlPlaneStore, LifecycleStore, MutationError, OperationInsert},
};

#[tokio::test]
async fn retirement_waits_for_fleet_generations_and_open_effects_then_survives_restart(
) -> TestResult {
    let fixture = fixture().await?;
    retire(&fixture).await?;
    fixture.store.periodic_scan(11).await?;
    status(&fixture, "Retiring").await?;
    fixture.seed(G::Idle).await?;
    fixture.store.fleet_set_tombstone("fleet", 12).await?;
    fixture.store.periodic_scan(13).await?;
    status(&fixture, "Retiring").await?;
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
            now: 14,
        })
        .await?;
    for state in [G::Retiring, G::DestroyPending, G::Destroying, G::Destroyed] {
        fixture
            .store
            .generation_advance("generation", state, 15)
            .await?;
    }
    fixture.store.periodic_scan(16).await?;
    status(&fixture, "Retiring").await?;
    fixture
        .store
        .operation_update_state("late-effect", "Completed", 17)
        .await?;
    let reopened = SqliteControlPlane::new(
        Store::open(&fixture.directory.path().join("state.db")).await?,
        fixture.directory.path().join("artifacts"),
    );
    assert_eq!(reopened.periodic_scan(18).await?.profiles_retired, 2);
    status(&fixture, "Retired").await?;
    for key in ["profile", "forgejo"] {
        assert_eq!(
            reopened
                .profile_change_get(&format!("retire-{key}"))
                .await?
                .ok_or("missing change")?
                .state,
            "Succeeded"
        );
    }
    assert_eq!(reopened.periodic_scan(19).await?.profiles_retired, 0);
    assert!(reopened
        .template_protected_bindings("profile", 1)
        .await?
        .is_some());
    Ok(())
}

#[tokio::test]
async fn retirement_completion_rolls_back_heads_when_change_commit_fails() -> TestResult {
    let fixture = fixture().await?;
    fixture.store.fleet_set_tombstone("fleet", 2).await?;
    retire(&fixture).await?;
    fixture.store.store().connection().execute_unprepared(
        "CREATE TRIGGER fail_retirement BEFORE UPDATE ON profile_changes WHEN NEW.state='Succeeded'
        BEGIN SELECT RAISE(ABORT, 'injected retirement failure'); END;"
    ).await?;
    assert!(fixture.store.periodic_scan(11).await.is_err());
    status(&fixture, "Retiring").await?;
    fixture
        .store
        .store()
        .connection()
        .execute_unprepared("DROP TRIGGER fail_retirement")
        .await?;
    fixture.store.periodic_scan(12).await?;
    status(&fixture, "Retired").await
}

#[tokio::test]
async fn pool_commit_rechecks_profile_retirement_after_admission() -> TestResult {
    let fixture = fixture().await?;
    let admitted = pool(&fixture)?;
    retire(&fixture).await?;
    assert!(matches!(
        fixture
            .store
            .commit_template_pool_mutation(admitted)
            .await?,
        Err(MutationError::RetirementBlocked { .. })
    ));
    assert!(fixture.store.template_pool_get("pool").await?.is_none());
    Ok(())
}

#[tokio::test]
async fn active_pool_keeps_template_reference_but_its_tombstone_does_not() -> TestResult {
    let fixture = fixture().await?;
    let mut pool = pool(&fixture)?;
    assert!(fixture
        .store
        .commit_template_pool_mutation(pool.clone())
        .await?
        .is_ok());
    fixture.store.fleet_set_tombstone("fleet", 2).await?;
    retire(&fixture).await?;
    fixture.store.periodic_scan(11).await?;
    assert_eq!(
        fixture
            .store
            .template_profile_get("profile")
            .await?
            .ok_or("missing template")?
            .status,
        "Retiring"
    );
    assert_eq!(
        fixture
            .store
            .auth_profile_get("forgejo")
            .await?
            .ok_or("missing auth")?
            .status,
        "Retired"
    );
    pool.revision = 2;
    pool.change.id = "pool-delete".into();
    pool.change.kind = "Delete".into();
    assert!(fixture
        .store
        .commit_template_pool_delete(pool)
        .await?
        .is_ok());
    fixture.store.periodic_scan(12).await?;
    status(&fixture, "Retired").await
}

#[tokio::test]
async fn validation_claim_blocks_retirement_until_its_fence_expires() -> TestResult {
    let fixture = fixture().await?;
    fixture.store.fleet_set_tombstone("fleet", 2).await?;
    retire(&fixture).await?;
    fixture.store.store().connection().execute_unprepared(
        "INSERT INTO profile_changes (id,resource_kind,profile_key,revision,kind,state,attempts,lease_owner,lease_expires_at,created_at,updated_at)
        VALUES ('validation','template_profile','profile',1,'Publish','Running',1,'validator',100,1,1)"
    ).await?;
    fixture.store.periodic_scan(99).await?;
    assert_eq!(
        fixture
            .store
            .template_profile_get("profile")
            .await?
            .ok_or("missing template")?
            .status,
        "Retiring"
    );
    fixture.store.periodic_scan(100).await?;
    status(&fixture, "Retired").await?;
    assert_eq!(
        fixture
            .store
            .profile_change_get("validation")
            .await?
            .ok_or("missing validation")?
            .state,
        "Superseded"
    );
    Ok(())
}
