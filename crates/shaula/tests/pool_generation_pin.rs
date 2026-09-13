//! Regression: a fleet that references a shared `template_pool_ref`
//! (spec 0037) must reach pool member admission instead of erroring
//! "admitted fleet revision has no resolved template pin" — the create
//! gate used to check only the legacy inline `template_pool`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "common/auth_operation_fixture.rs"]
mod fixture;

use fixture::Harness;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::registry::LifecycleStore;
use std::sync::atomic::Ordering;

/// A `template_pool_ref` fleet with demand draws a pool member: the
/// generation is created carrying the frozen `pool_member_key`, and the
/// reconcile never trips the resolved-pin gate.
#[tokio::test]
async fn shared_pool_fleet_draws_a_member_on_create() {
    let harness = Harness::new_with_pool().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);
    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness.store.demand_snapshot("f1", 1, 10).await.unwrap();
    let report = supervisor.tick(20).await.unwrap();
    // The pool member draw produced a Create; the pin gate did not error.
    assert!(!report.blocked, "reconcile blocked: {:?}", report.reason);
    assert_eq!(report.created, 1);
    let generation = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .pop()
        .expect("a generation was admitted from the pool");
    assert_eq!(generation.pool_member_key.as_deref(), Some("only"));
    assert_eq!(generation.template_profile_key, "k8s-linux");
    assert_eq!(generation.template_revision, 1);
    assert_eq!(generation.state, G::WaitingOnline);
    assert_eq!(harness.runtime.creates.load(Ordering::SeqCst), 1);
}
