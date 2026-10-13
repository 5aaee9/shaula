//! A Create that fails in its local, pre-JIT stages (workspace preparation)
//! has crossed no remote effect. It must not linger in `Creating`: that state
//! counts as occupancy, so a burst of failed prepares would otherwise satisfy
//! demand with phantom runners until a daemon restart recovered them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "common/auth_operation_fixture.rs"]
mod fixture;

use fixture::Harness;
use shaula_core::{lifecycle::GenerationState as G, registry::LifecycleStore};
use shaula_daemon::supervisor::LifecycleLimits;
use std::sync::{atomic::Ordering, Arc};

const DEMAND: i64 = 3;
/// Past the one-tick CleanupRequired grace (spec 0024 §2.1).
const AFTER_GRACE: i64 = 20 + 60_001;

async fn states(harness: &Harness) -> Vec<G> {
    let mut states: Vec<G> = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.state)
        .collect();
    states.sort_by_key(|s| s.as_str_repr());
    states
}

#[tokio::test]
async fn failed_workspace_prepare_releases_capacity_without_a_restart() {
    let harness = Harness::new().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);
    harness.runtime.prepare_fails.store(true, Ordering::SeqCst);
    let limits = LifecycleLimits {
        create: Arc::new(tokio::sync::Semaphore::new(8)),
        destroy: Arc::new(tokio::sync::Semaphore::new(1)),
        template_create: Default::default(),
    };
    let supervisor = harness.supervisor_with(1, limits, DEMAND);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness
        .store
        .demand_snapshot("f1", DEMAND, 10)
        .await
        .unwrap();
    let _ = supervisor.tick(20).await;

    let failed = states(&harness).await;
    assert_eq!(failed.len(), DEMAND as usize);
    assert!(
        failed.iter().all(|s| *s == G::CleanupRequired),
        "a pre-JIT failure leaves no generation in Creating: {failed:?}"
    );
    assert_eq!(harness.runtime.creates.load(Ordering::SeqCst), 0);

    // Preparation recovers. The failed generations retire as never started
    // and the same demand is served by fresh generations.
    harness.runtime.prepare_fails.store(false, Ordering::SeqCst);
    for step in 0..3 {
        supervisor.tick(AFTER_GRACE + step * 1_000).await.unwrap();
    }
    let recovered = states(&harness).await;
    assert_eq!(
        recovered.iter().filter(|s| **s == G::Destroyed).count(),
        DEMAND as usize,
        "{recovered:?}"
    );
    assert_eq!(
        recovered.iter().filter(|s| **s == G::WaitingOnline).count(),
        DEMAND as usize,
        "{recovered:?}"
    );
    assert_eq!(
        harness.runtime.creates.load(Ordering::SeqCst),
        DEMAND as usize
    );
    assert_eq!(harness.runtime.destroys.load(Ordering::SeqCst), 0);
}
