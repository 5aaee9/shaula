//! Actual supervisor producers and SQLite retention, without rewriting operation states.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "common/auth_operation_fixture.rs"]
mod fixture;

use fixture::Harness;
use shaula_core::{
    lifecycle::GenerationState as G,
    registry::{AuthExecutionStore, LifecycleStore},
};
use std::sync::atomic::Ordering;

async fn create(harness: &Harness) -> shaula_core::registry::GenerationRecord {
    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness.store.demand_snapshot("f1", 1, 10).await.unwrap();
    supervisor.tick(20).await.unwrap();
    harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .unwrap()
}

#[tokio::test]
async fn definite_jit_and_apply_release_old_auth_only_after_real_destroy() {
    let harness = Harness::new().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);
    let generation = create(&harness).await;
    assert_eq!(generation.state, G::WaitingOnline);
    assert_eq!(generation.github_runner_id, Some(77));
    assert!(harness
        .store
        .operations_open_for_generation(&generation.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(harness.runtime.creates.load(Ordering::SeqCst), 1);
    // Simulate the listener's online event, then rotate while the runner exists.
    harness
        .store
        .generation_advance(&generation.id, G::Idle, 21)
        .await
        .unwrap();
    let supervisor = harness.rotate().await;
    assert_eq!(
        harness.store.auth_execution_refs("f1").await.unwrap(),
        vec![("prod-app".into(), 1), ("prod-app".into(), 2)]
    );
    harness.store.demand_snapshot("f1", 0, 31).await.unwrap();
    assert_eq!(supervisor.tick(40).await.unwrap().destroyed, 1);
    assert_eq!(
        harness
            .store
            .generation_get(&generation.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        G::Destroyed
    );
    assert!(harness
        .store
        .operations_open_for_generation(&generation.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        harness.store.auth_execution_refs("f1").await.unwrap(),
        vec![("prod-app".into(), 2)]
    );
    assert_eq!(harness.runtime.destroys.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn uncertain_jit_and_access_error_keep_original_auth_and_never_repost() {
    for uncertain in [true, false] {
        let harness = Harness::new().await;
        harness
            .github
            .jit_uncertain
            .store(uncertain, Ordering::SeqCst);
        let generation = create(&harness).await;
        assert_eq!(generation.state, G::Quarantined);
        let open = harness
            .store
            .operations_open_for_generation(&generation.id)
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(
            (open[0].kind.as_str(), open[0].state.as_str()),
            ("JitStarting", "Pending")
        );
        let supervisor = harness.rotate().await;
        supervisor.tick(40).await.unwrap();
        assert!(harness
            .store
            .auth_execution_refs("f1")
            .await
            .unwrap()
            .contains(&("prod-app".into(), 1)));
        assert_eq!(harness.github.effects.load(Ordering::SeqCst), 1);
        assert_eq!(harness.runtime.creates.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn uncertain_create_keeps_apply_intent_but_completes_definite_jit() {
    let harness = Harness::new().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);
    harness
        .runtime
        .create_uncertain
        .store(true, Ordering::SeqCst);
    let generation = create(&harness).await;
    assert_eq!(generation.state, G::CleanupRequired);
    let open = harness
        .store
        .operations_open_for_generation(&generation.id)
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        (open[0].kind.as_str(), open[0].state.as_str()),
        ("Create", "ApplyStarting")
    );
    harness.rotate().await;
    assert!(harness
        .store
        .auth_execution_refs("f1")
        .await
        .unwrap()
        .contains(&("prod-app".into(), 1)));
}

#[tokio::test]
async fn successful_destroy_does_not_complete_an_earlier_uncertain_attempt() {
    let harness = Harness::new().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);
    let generation = create(&harness).await;
    harness
        .store
        .generation_advance(&generation.id, G::Idle, 21)
        .await
        .unwrap();
    let supervisor = harness.rotate().await;
    harness.store.demand_snapshot("f1", 0, 31).await.unwrap();
    harness
        .runtime
        .destroy_uncertain_once
        .store(true, Ordering::SeqCst);
    assert_eq!(supervisor.tick(40).await.unwrap().destroyed, 0);
    let before = harness
        .store
        .operations_open_for_generation(&generation.id)
        .await
        .unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].kind, "Destroy");
    assert_eq!(supervisor.tick(50).await.unwrap().destroyed, 1);
    let after = harness
        .store
        .operations_open_for_generation(&generation.id)
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].id, before[0].id,
        "only the current successful attempt may close"
    );
    assert!(harness
        .store
        .auth_execution_refs("f1")
        .await
        .unwrap()
        .contains(&("prod-app".into(), 1)));
}
