// Existing shared HTTP fixtures use assertion-style unwraps in test-only code.
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[allow(clippy::unwrap_used)]
#[path = "common/lifecycle_fakes.rs"]
mod fakes;
#[path = "support/runner_lifetime_fixture.rs"]
mod fixture;

use fixture::{Fixture, TestResult};
use shaula_core::{
    lifecycle::GenerationState as G,
    registry::{ControlPlaneStore, LifecycleStore},
    runner_lifetime::RunnerLifetimeStore,
};
use std::{sync::atomic::Ordering, time::Duration};

#[tokio::test]
async fn hard_timeout_terminates_busy_before_removal_and_resumes_without_redestroy() -> TestResult {
    let f = Fixture::new().await?;
    f.seed(true).await?;
    f.github.busy.store(true, Ordering::SeqCst);
    f.github.denied.store(true, Ordering::SeqCst);
    // Handoff/auth readiness does not block hard resource termination.
    let supervisor = f
        .supervisor(Duration::from_secs(10))?
        .with_execution_ready(false);
    f.clock.0.store(10_999, Ordering::SeqCst);
    supervisor.tick(10_999).await?;
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 0);
    f.clock.0.store(11_000, Ordering::SeqCst);
    supervisor.tick(11_000).await?;
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(f.github.removals.load(Ordering::SeqCst), 1);
    assert_eq!(f.state().await?, G::Destroying);
    assert_eq!(f.store.capacity_counters("f1").await?, (0, 1));
    assert!(f
        .store
        .generation_lifetime("gen")
        .await?
        .resources_destroyed_at
        .is_some());
    // A rebuilt supervisor with a longer policy must still finish forced cleanup.
    let restarted = f
        .supervisor(Duration::from_secs(7200))?
        .with_execution_ready(false);
    restarted.tick(11_001).await?;
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    assert_eq!(f.state().await?, G::Destroying);
    f.github.busy.store(false, Ordering::SeqCst);
    assert_eq!(restarted.tick(11_002).await?.destroyed, 1);
    assert_eq!(f.state().await?, G::Destroyed);
    assert_eq!(f.store.capacity_counters("f1").await?, (0, 0));
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn hard_timeout_retries_failed_resource_destruction_without_removing_runner() -> TestResult {
    let f = Fixture::new().await?;
    f.seed(true).await?;
    f.runtime.fail.store(true, Ordering::SeqCst);
    f.clock.0.store(11_000, Ordering::SeqCst);
    let supervisor = f
        .supervisor(Duration::from_secs(10))?
        .with_execution_ready(false);
    supervisor.tick(11_000).await?;
    assert_eq!(f.state().await?, G::DestroyPending);
    assert_eq!(f.github.removals.load(Ordering::SeqCst), 0);
    f.runtime.fail.store(false, Ordering::SeqCst);
    assert_eq!(supervisor.tick(11_001).await?.destroyed, 1);
    assert_eq!(f.state().await?, G::Destroyed);
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn hard_timeout_preserves_ownership_and_quarantine_guards() -> TestResult {
    let f = Fixture::new().await?;
    f.seed(false).await?;
    let supervisor = f
        .supervisor(Duration::from_secs(10))?
        .with_execution_ready(false);
    supervisor.tick(11_000).await?;
    assert_eq!(f.state().await?, G::Quarantined);
    supervisor.tick(i64::MAX).await?;
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 0);
    assert_eq!(f.github.removals.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn stale_supervisor_cannot_start_hard_timeout() -> TestResult {
    let f = Fixture::new().await?;
    f.seed(true).await?;
    let mut guard = shaula_core::registry::FleetRuntimeGuard::from(
        &f.store.fleet_get("f1").await?.ok_or("head")?,
    );
    guard.mutation_fence += 1;
    f.supervisor(Duration::from_secs(10))?
        .with_runtime_guard(guard)
        .tick(11_000)
        .await?;
    assert_eq!(f.state().await?, G::Busy);
    assert_eq!(f.runtime.destroys.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn github_create_starts_lifetime_after_slow_provisioning() -> TestResult {
    let f = Fixture::new().await?;
    f.github.jit_definite.store(true, Ordering::SeqCst);
    f.store.demand_snapshot("f1", 1, 1000).await?;
    let supervisor = f.supervisor(Duration::from_secs(7200))?;
    supervisor.tick(1000).await?; // settle handoff
    assert_eq!(supervisor.tick(1000).await?.created, 1);
    let generations = f.store.generations_for_fleet("f1").await?;
    assert_eq!(generations.len(), 1);
    assert_eq!(generations[0].created_at, 1000);
    assert_eq!(
        f.store
            .generation_lifetime(&generations[0].id)
            .await?
            .provisioned_at,
        Some(6000)
    );
    Ok(())
}
