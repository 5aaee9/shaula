//! Regression: a fleet that references a shared `template_pool_ref`
//! (spec 0037) must reach pool member admission instead of erroring
//! "admitted fleet revision has no resolved template pin" — the create
//! gate used to check only the legacy inline `template_pool`. A pool change
//! then rolls out while the Fleet is occupied (spec 0037 §5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "common/auth_operation_fixture.rs"]
mod fixture;

use common::authorized;
use fixture::Harness;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::registry::FleetStore;
use shaula_core::registry::LifecycleStore;
use shaula_daemon::supervisor::LifecycleLimits;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tower::ServiceExt;

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

/// Spec 0037 §5: one runner is admitted under pool revision 1, then a pool
/// PUT mints revision 2 while that runner occupies the Fleet. The next
/// cascade pass moves the Fleet at once; the runner keeps the pins it was
/// created with. Returns the harness and that runner's Generation ID.
async fn rolled_out_while_occupied(state: G) -> (Harness, String) {
    let harness = Harness::new_with_pool().await;
    harness.github.jit_definite.store(true, Ordering::SeqCst);
    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(9).await.unwrap().handoff_acknowledged);
    harness.store.demand_snapshot("f1", 1, 10).await.unwrap();
    assert_eq!(supervisor.tick(20).await.unwrap().created, 1);
    let old = harness.store.generations_for_fleet("f1").await.unwrap();
    let old = old.first().expect("one runner admitted").id.clone();
    for runner in harness.github.runners.lock().unwrap().iter_mut() {
        runner.status = "online".into();
    }
    harness
        .store
        .generation_advance(&old, G::Idle, 30)
        .await
        .unwrap();
    if state == G::Busy {
        harness
            .store
            .generation_advance(&old, G::Busy, 31)
            .await
            .unwrap();
    }
    let weighted = r#"{
        "members": [
            {"key":"only","template_profile_ref":"k8s-linux","weight":2,"max_runners":1,"template_inputs":{}}
        ],
        "failure_policy":"backpressure"
    }"#;
    let current = harness
        .app
        .clone()
        .oneshot(authorized("GET", "/api/v1/template-pools/builders", None))
        .await
        .unwrap();
    let etag = current.headers().get("etag").unwrap().clone();
    let mut update = authorized(
        "PUT",
        "/api/v1/template-pools/builders",
        Some(weighted.into()),
    );
    update.headers_mut().remove("if-none-match");
    update.headers_mut().insert("if-match", etag);
    let status = harness.app.clone().oneshot(update).await.unwrap().status();
    assert_eq!(status, axum::http::StatusCode::ACCEPTED);
    assert_eq!(
        harness
            .service
            .cascade_template_follow_upgrades(40)
            .await
            .unwrap(),
        1,
        "an occupied Fleet catches up without waiting to drain"
    );
    let head = harness
        .store
        .fleet_revision_latest("f1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.revision, 2);
    assert_eq!(head.template_pool_ref, Some(("builders".into(), 2)));
    (harness, old)
}

async fn assert_pins_kept(harness: &Harness, id: &str) {
    let generation = harness.store.generation_get(id).await.unwrap().unwrap();
    assert_eq!(generation.fleet_revision, 1);
    assert_eq!(generation.pool_member_key.as_deref(), Some("only"));
    assert_eq!(generation.template_revision, 1);
}

#[tokio::test]
async fn pool_change_retires_an_idle_runner_of_the_old_revision() {
    let (harness, old) = rolled_out_while_occupied(G::Idle).await;
    // The supervisor reads the Fleet head each tick. The first tick settles
    // the new revision's auth handoff; the next one reconciles under it.
    let supervisor = harness.supervisor(1);
    assert!(supervisor.tick(50).await.unwrap().settled_this_tick);
    supervisor.tick(60).await.unwrap();
    assert_eq!(
        harness
            .store
            .generation_get(&old)
            .await
            .unwrap()
            .unwrap()
            .state,
        G::Destroyed,
        "Idle under the old pool revision is retired even with demand"
    );
    assert_eq!(harness.github.removals.load(Ordering::SeqCst), 1);
    assert_eq!(harness.runtime.destroys.load(Ordering::SeqCst), 1);
    assert_pins_kept(&harness, &old).await;
    // The freed slot is refilled under the current pool revision.
    assert_eq!(supervisor.tick(70).await.unwrap().created, 1);
    let replacement = harness
        .store
        .generations_for_fleet("f1")
        .await
        .unwrap()
        .into_iter()
        .find(|g| g.id != old)
        .expect("a replacement was admitted");
    assert_eq!(replacement.fleet_revision, 2);
    assert_eq!(replacement.pool_member_key.as_deref(), Some("only"));
}

#[tokio::test]
async fn pool_change_lets_a_busy_runner_finish_its_job() {
    let (harness, old) = rolled_out_while_occupied(G::Busy).await;
    // The member cap counts its runners across pool revisions: the Busy
    // revision-1 runner still fills `only` (max_runners 1), so more demand
    // and Fleet headroom admit nothing until it finishes.
    let limits = LifecycleLimits {
        create: Arc::new(tokio::sync::Semaphore::new(1)),
        destroy: Arc::new(tokio::sync::Semaphore::new(1)),
        template_create: Default::default(),
    };
    let supervisor = harness.supervisor_with(1, limits, 2);
    harness.store.demand_snapshot("f1", 2, 45).await.unwrap();
    assert!(supervisor.tick(50).await.unwrap().settled_this_tick);
    assert_eq!(supervisor.tick(60).await.unwrap().created, 0);
    assert_eq!(harness.runtime.creates.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness
            .store
            .generation_get(&old)
            .await
            .unwrap()
            .unwrap()
            .state,
        G::Busy
    );
    assert_eq!(harness.github.removals.load(Ordering::SeqCst), 0);
    assert_pins_kept(&harness, &old).await;
}
