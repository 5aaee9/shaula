//! Exercise the real SQLite checkpoints and Forgejo supervisor with faulted ports.

use super::forgejo_pool_support::{Fixture, TestResult};
use crate::{registry_impl::SqliteControlPlane, Store};
use shaula_core::{
    lifecycle::GenerationState as G, registry::LifecycleStore, runner_lifetime::RunnerLifetimeStore,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

async fn busy() -> TestResult<Fixture> {
    let fixture = Fixture::new(1, 1).await?;
    fixture.supervisor().await?.tick().await?;
    fixture.declare("idle")?;
    fixture.supervisor().await?.tick().await?;
    fixture.declare("active")?;
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.generation().await?.state, G::Busy);
    Ok(fixture)
}

#[tokio::test]
async fn default_lifetime_expires_busy_at_exact_boundary_even_with_stale_demand() -> TestResult {
    let f = busy().await?;
    f.forgejo.fail_jobs.store(true, Ordering::SeqCst);
    f.clock.0.store(7_200_999, Ordering::SeqCst);
    f.supervisor().await?.tick().await?;
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 0);
    f.clock.0.store(7_201_000, Ordering::SeqCst);
    let report = f.supervisor().await?.tick().await?;
    assert!(report.stale_demand);
    assert_eq!(report.destroyed, 1);
    assert_eq!(f.generation().await?.state, G::Destroyed);
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(f.forgejo.deletes.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn unavailable_registration_cleanup_survives_reopen_and_policy_change() -> TestResult {
    let mut f = busy().await?;
    let id = f.generation().await?.id;
    f.forgejo.fail_inventory.store(true, Ordering::SeqCst);
    f.clock.0.store(11_000, Ordering::SeqCst);
    let report = f
        .supervisor()
        .await?
        .with_runner_max_lifetime(Duration::from_secs(10))
        .tick()
        .await?;
    assert!(report.stale_inventory);
    assert_eq!(report.destroyed, 0);
    assert_eq!(f.generation().await?.state, G::Destroying);
    let checkpoint = f.store.generation_lifetime(&id).await?;
    assert_eq!(checkpoint.provisioned_at, Some(1000));
    assert_eq!(checkpoint.expiry_requested_at, Some(11_000));
    assert_eq!(checkpoint.resources_destroyed_at, Some(11_000));

    // Reopen the durable database and rebuild the supervisor, with a LONGER
    // policy. Persisted force intent must still retry deregistration only.
    let reopened = Store::open(&f.directory.path().join("state.db")).await?;
    reopened.migrate().await?;
    f.store = Arc::new(SqliteControlPlane::new(
        reopened,
        f.directory.path().join("artifacts"),
    ));
    f.supervisor().await?.tick().await?;
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    f.forgejo.fail_inventory.store(false, Ordering::SeqCst);
    f.forgejo.fail_jobs.store(true, Ordering::SeqCst);
    f.forgejo.fail_delete.store(true, Ordering::SeqCst);
    f.supervisor().await?.tick().await?;
    assert_eq!(f.generation().await?.state, G::Destroying);
    f.forgejo.fail_delete.store(false, Ordering::SeqCst);
    assert_eq!(f.supervisor().await?.tick().await?.destroyed, 1);
    assert_eq!(f.generation().await?.state, G::Destroyed);
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(f.store.generation_lifetime(&id).await?, checkpoint);
    Ok(())
}

#[tokio::test]
async fn failed_destroy_retries_before_deleting_registration() -> TestResult {
    let f = busy().await?;
    f.forgejo.fail_jobs.store(true, Ordering::SeqCst);
    f.runtime.fail_destroy.store(true, Ordering::SeqCst);
    f.clock.0.store(7_201_000, Ordering::SeqCst);
    f.supervisor().await?.tick().await?;
    assert_eq!(f.generation().await?.state, G::DestroyPending);
    assert_eq!(f.forgejo.deletes.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.store
            .generation_lifetime(&f.generation().await?.id)
            .await?
            .resources_destroyed_at,
        None
    );
    f.runtime.fail_destroy.store(false, Ordering::SeqCst);
    assert_eq!(f.supervisor().await?.tick().await?.destroyed, 1);
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 2);
    assert_eq!(f.forgejo.deletes.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn hard_timeout_never_deletes_a_reused_remote_identity() -> TestResult {
    let f = busy().await?;
    f.forgejo.runners.lock().map_err(|_| "poisoned")?[0].uuid = "foreign".into();
    f.forgejo.fail_jobs.store(true, Ordering::SeqCst);
    f.clock.0.store(7_201_000, Ordering::SeqCst);
    f.supervisor().await?.tick().await?;
    // Owned infra is terminated, but a conflicting registry object is untouched.
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(f.forgejo.deletes.load(Ordering::SeqCst), 0);
    assert_eq!(f.generation().await?.state, G::Quarantined);
    f.supervisor().await?.tick().await?;
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn successful_create_clock_is_write_once_and_does_not_start_at_allocation() -> TestResult {
    let f = Fixture::new(0, 1).await?;
    let generation = f.seed(G::Creating).await?;
    assert_eq!(
        f.store
            .generation_lifetime(&generation.id)
            .await?
            .provisioned_at,
        None
    );
    assert!(
        !f.store
            .generation_request_expiry(&generation.id, 100_000_000)
            .await?
    );
    f.store
        .generation_set_result(&generation.id, "{}", "digest", 90_000)
        .await?;
    f.store
        .generation_advance(&generation.id, G::WaitingOnline, 91_000)
        .await?;
    f.store
        .generation_set_result(&generation.id, "{}", "digest", 99_000)
        .await?;
    let lifetime = f.store.generation_lifetime(&generation.id).await?;
    assert_eq!(lifetime.provisioned_at, Some(90_000));
    assert!(!lifetime.is_due(89_000, Duration::from_secs(1)));
    assert!(!lifetime.is_due(90_999, Duration::from_secs(1)));
    assert!(lifetime.is_due(91_000, Duration::from_secs(1)));
    Ok(())
}
