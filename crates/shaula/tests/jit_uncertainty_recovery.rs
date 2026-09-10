//! spec 0025: an Uncertain JIT mint is classified by exact-name lookup
//! instead of orphaning the landed runner entity.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "common/auth_operation_fixture.rs"]
mod fixture;

use shaula_core::lifecycle::GenerationState as G;
use shaula_core::registry::LifecycleStore;
use std::sync::atomic::Ordering;

use fixture::Harness;

#[tokio::test]
async fn uncertain_landed_entity_is_removed_and_generation_routes_to_cleanup() {
    let harness = Harness::new().await;
    harness.github.jit_uncertain.store(true, Ordering::SeqCst);
    harness
        .github
        .jit_uncertain_lands
        .store(true, Ordering::SeqCst);

    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness.store.demand_snapshot("f1", 1, 10).await.unwrap();
    supervisor.tick(20).await.unwrap();

    let generation = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(generation.state, G::CleanupRequired);
    // The landed entity was recorded and converged away: no orphan
    // inventory remains to trip UnknownRemoteRunner.
    assert_eq!(generation.github_runner_id, Some(78));
    assert_eq!(harness.github.removals.load(Ordering::SeqCst), 1);
    assert!(harness.github.runners.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uncertain_absent_entity_is_proven_resource_free_and_cleans_up() {
    let harness = Harness::new().await;
    harness.github.jit_uncertain.store(true, Ordering::SeqCst);

    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness.store.demand_snapshot("f1", 1, 10).await.unwrap();
    supervisor.tick(20).await.unwrap();

    let generation = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(generation.state, G::CleanupRequired);
    assert_eq!(generation.github_runner_id, None);
    assert_eq!(harness.github.removals.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn uncertain_with_unavailable_classification_stays_quarantined() {
    let harness = Harness::new().await;
    harness.github.jit_uncertain.store(true, Ordering::SeqCst);
    harness
        .github
        .runner_lookup_denied
        .store(true, Ordering::SeqCst);

    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness.store.demand_snapshot("f1", 1, 10).await.unwrap();
    supervisor.tick(20).await.unwrap();

    let generation = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(generation.state, G::Quarantined);
    assert_eq!(harness.github.removals.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cleanup_required_generation_is_destroyed_after_grace_not_stranded() {
    // Spec 0024 §2.1: a CleanupRequired generation past the one-tick
    // grace drives the normal destroy chain (runner removal included)
    // instead of being stranded for quarantine. The generation reaches
    // CleanupRequired through the readiness timeout after a COMPLETE
    // create (provenance intact) — the 2026-09-10 23:24 incident shape.
    let harness = Harness::new().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);

    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness.store.demand_snapshot("f1", 1, 10).await.unwrap();
    supervisor.tick(20).await.unwrap();

    let generation = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(generation.state, G::WaitingOnline);

    // The runner never comes online in the inventory and the readiness
    // timeout expires (spec 0024 §1); past the 60s cleanup grace the
    // destroy chain converges the generation.
    let late = 20 + shaula_daemon::supervisor::READINESS_TIMEOUT_MS + 1_000;
    supervisor.tick(late).await.unwrap();
    // The 60s cleanup grace runs from the CleanupRequired transition;
    // a subsequent tick drives the destroy chain.
    supervisor.tick(late + 61_000).await.unwrap();
    let generation = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(generation.state, G::Destroyed);
    assert_eq!(harness.runtime.destroys.load(Ordering::SeqCst), 1);
}
