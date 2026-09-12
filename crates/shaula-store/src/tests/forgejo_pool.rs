//! SQLite-backed Forgejo lifecycle regressions, independent of GitHub fixtures.

use super::forgejo_pool_support::{Fixture, TestResult};
use sea_orm::ConnectionTrait;
use shaula_core::{
    lifecycle::GenerationState as G,
    registry::{ControlPlaneStore, LifecycleStore, OperationInsert},
};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn new_registration_is_not_destroyed_by_its_pre_create_inventory() -> TestResult {
    let fixture = Fixture::new(1, 1).await?;
    let report = fixture.supervisor().await?.tick().await?;
    assert_eq!((report.created, report.destroyed), (1, 0));
    assert_eq!(fixture.generation().await?.state, G::WaitingOnline);
    assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runtime.destroys.load(Ordering::SeqCst), 0);
    assert!(fixture.store.scale_set_get("fleet").await?.is_none());
    assert!(fixture
        .store
        .operations_open_for_generation(&fixture.generation().await?.id)
        .await?
        .is_empty());
    Ok(())
}

#[tokio::test]
async fn idle_readiness_busy_and_ephemeral_completion_use_inventory() -> TestResult {
    let fixture = Fixture::new(0, 1).await?;
    fixture.forgejo.waiting.store(1, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    fixture.declare("idle")?;
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.generation().await?.state, G::Idle);
    assert_eq!(
        fixture
            .store
            .fleet_get("fleet")
            .await?
            .ok_or("missing head")?
            .phase,
        "Ready"
    );
    fixture.declare("active")?;
    fixture.forgejo.waiting.store(0, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.generation().await?.state, G::Busy);
    assert_eq!(fixture.forgejo.deletes.load(Ordering::SeqCst), 0);
    fixture
        .forgejo
        .runners
        .lock()
        .map_err(|_| "poisoned")?
        .clear();
    let report = fixture.supervisor().await?.tick().await?;
    assert_eq!(report.destroyed, 1);
    assert_eq!(fixture.generation().await?.state, G::Destroyed);
    assert_eq!(fixture.runtime.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.forgejo.deletes.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn idle_inventory_alone_does_not_authorize_deletion() -> TestResult {
    let fixture = Fixture::new(0, 1).await?;
    fixture.forgejo.waiting.store(1, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    fixture.declare("idle")?;
    fixture.forgejo.waiting.store(0, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.generation().await?.state, G::Idle);
    assert_eq!(fixture.forgejo.deletes.load(Ordering::SeqCst), 0);
    fixture.runtime.idle_proof.store(true, Ordering::SeqCst);
    assert_eq!(fixture.supervisor().await?.tick().await?.destroyed, 1);
    assert_eq!(fixture.forgejo.deletes.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn failed_jobs_preserve_value_and_timestamp_and_freeze_effects() -> TestResult {
    let fixture = Fixture::new(0, 2).await?;
    fixture.forgejo.waiting.store(1, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    fixture.declare("idle")?;
    let previous = fixture
        .store
        .store()
        .demand_get("fleet")
        .await?
        .ok_or("demand")?;
    fixture.clock.0.store(2_000, Ordering::SeqCst);
    fixture.forgejo.fail_jobs.store(true, Ordering::SeqCst);
    fixture.runtime.idle_proof.store(true, Ordering::SeqCst);
    let report = fixture.supervisor().await?.tick().await?;
    assert!(report.stale_demand);
    assert_eq!((report.created, report.destroyed), (0, 0));
    let after = fixture
        .store
        .store()
        .demand_get("fleet")
        .await?
        .ok_or("demand")?;
    assert_eq!(
        (after.total_assigned_jobs, after.updated_at),
        (previous.total_assigned_jobs, previous.updated_at)
    );
    Ok(())
}

#[tokio::test]
async fn quarantine_holds_capacity_even_with_no_remote_inventory() -> TestResult {
    let fixture = Fixture::new(1, 1).await?;
    fixture.seed(G::Quarantined).await?;
    let report = fixture.supervisor().await?.tick().await?;
    assert_eq!(report.created, 0);
    assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn uncertain_undeclared_registration_is_quarantined_not_reposted() -> TestResult {
    let fixture = Fixture::new(1, 1).await?;
    fixture.forgejo.uncertain.store(true, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.generation().await?.state, G::Quarantined);
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.forgejo.deletes.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.creates.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn restart_classifies_open_registration_without_replaying_post() -> TestResult {
    let fixture = Fixture::new(0, 1).await?;
    let generation = fixture.seed(G::Creating).await?;
    fixture
        .store
        .operation_insert(OperationInsert {
            id: "registration".into(),
            generation_id: generation.id,
            kind: "ForgejoRegistration".into(),
            state: "Starting".into(),
            provenance_json: Some("{}".into()),
            saved_plan_path: None,
            saved_plan_digest: None,
            now: 1,
        })
        .await?;
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.generation().await?.state, G::Destroyed);
    assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.creates.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn identity_conflict_and_unknown_status_never_become_absence() -> TestResult {
    for conflict in [true, false] {
        let fixture = Fixture::new(1, 1).await?;
        fixture.supervisor().await?.tick().await?;
        fixture.declare("idle")?;
        {
            let mut runners = fixture.forgejo.runners.lock().map_err(|_| "poisoned")?;
            if conflict {
                runners[0].uuid = "other".into();
            } else {
                runners[0].status = "future".into();
            }
        }
        fixture.supervisor().await?.tick().await?;
        assert_eq!(fixture.generation().await?.state, G::Quarantined);
        assert_eq!(fixture.forgejo.deletes.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.runtime.destroys.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[tokio::test]
async fn stale_supervisor_cannot_register_after_fleet_fence_moves() -> TestResult {
    let fixture = Fixture::new(1, 1).await?;
    let stale = fixture.supervisor().await?;
    fixture
        .store
        .store()
        .connection()
        .execute_unprepared("UPDATE fleets SET mutation_fence=2")
        .await?;
    let report = stale.tick().await?;
    assert!(report.stale_inventory);
    assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn registration_identity_is_immutable_and_never_a_github_runner_id() -> TestResult {
    let fixture = Fixture::new(0, 1).await?;
    let generation = fixture.seed(G::Creating).await?;
    fixture
        .store
        .generation_set_forgejo_runner(&generation.id, 42, "uuid", 2)
        .await?;
    fixture
        .store
        .generation_set_forgejo_runner(&generation.id, 42, "uuid", 3)
        .await?;
    assert!(fixture
        .store
        .generation_set_forgejo_runner(&generation.id, 43, "uuid", 4)
        .await
        .is_err());
    assert!(fixture
        .store
        .generation_set_forgejo_runner(&generation.id, 42, "changed", 4)
        .await
        .is_err());
    assert!(fixture
        .store
        .generation_set_github_runner(&generation.id, 42, 4)
        .await
        .is_err());
    assert!(fixture.generation().await?.github_runner_id.is_none());
    Ok(())
}
