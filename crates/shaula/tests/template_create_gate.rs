//! Creates from one Template Profile are serialized when
//! `execution.template_create_concurrency` bounds them, while the global
//! create scheduler alone still lets one tick's budget start concurrently.
//! Proxmox locks the source template VM while cloning, so overlapping
//! clones fail instead of waiting, and a failed partial Create is quarantined.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "common/auth_operation_fixture.rs"]
mod fixture;

use fixture::Harness;
use shaula_core::{lifecycle::GenerationState as G, registry::LifecycleStore};
use shaula_daemon::{supervisor::LifecycleLimits, template_create_gate::TemplateCreateGates};
use std::sync::{atomic::Ordering, Arc};

const DEMAND: i64 = 3;

/// One scale-up of `DEMAND` runners with a create budget wide enough to run
/// them all at once. Returns the peak number of overlapping Create applies.
async fn scale_up(template_create_concurrency: Option<usize>) -> usize {
    let harness = Harness::new().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);
    harness.runtime.create_delay_ms.store(500, Ordering::SeqCst);
    let limits = LifecycleLimits {
        create: Arc::new(tokio::sync::Semaphore::new(8)),
        destroy: Arc::new(tokio::sync::Semaphore::new(1)),
        template_create: TemplateCreateGates::new(template_create_concurrency),
    };
    let supervisor = harness.supervisor_with(1, limits, DEMAND);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness
        .store
        .demand_snapshot("f1", DEMAND, 10)
        .await
        .unwrap();
    supervisor.tick(20).await.unwrap();

    let generations = harness.store.generations_for_fleet("f1").await.unwrap();
    assert_eq!(generations.len(), DEMAND as usize);
    assert!(
        generations.iter().all(|g| g.state == G::WaitingOnline),
        "every Create completes: {:?}",
        generations.iter().map(|g| g.state).collect::<Vec<_>>()
    );
    assert_eq!(
        harness.runtime.creates.load(Ordering::SeqCst),
        DEMAND as usize
    );
    harness
        .runtime
        .peak_creates_in_flight
        .load(Ordering::SeqCst)
}

#[tokio::test]
async fn template_create_concurrency_serializes_applies_of_one_template() {
    assert_eq!(scale_up(Some(1)).await, 1);
}

#[tokio::test]
async fn unbounded_template_creates_still_overlap() {
    assert!(scale_up(None).await > 1);
}
