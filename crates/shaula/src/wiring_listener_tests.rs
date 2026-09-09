//! Composition regressions for the production Pending-to-Ready path.

#[path = "wiring_labels_fence_tests.rs"]
mod labels_fence_tests;
#[path = "wiring_labels_tests.rs"]
mod labels_tests;
#[path = "wiring_listener_replacement_tests.rs"]
mod replacement_tests;

use super::*;
use crate::auth_worker_mock::{endpoints, listener::ListenerMock, mock_server_cfg, MockConfig};
use shaula_core::registry::LifecycleStore;
use std::sync::atomic::{AtomicI64, Ordering};

struct TestClock(AtomicI64);
impl shaula_core::ports::Clock for TestClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

const NOW: i64 = 1_800_000_000_000;

async fn setup() -> (
    TestPlane,
    SupervisorWiring,
    Arc<ListenerMock>,
    Arc<TestClock>,
    crate::auth_worker_mock::Mock,
) {
    let listener = Arc::new(ListenerMock::default());
    let mock = mock_server_cfg(MockConfig {
        listener: Some(listener.clone()),
        ..Default::default()
    })
    .await;
    let plane = test_plane().await;
    // Zero capacity is a valid paused fleet. Listener reconciliation still
    // observes GitHub's existing assigned demand, without provisioning effects.
    seed_profile_and_fleet_config(&plane, &[], 0).await;
    let clock = Arc::new(TestClock(AtomicI64::new(NOW)));
    let mut wiring = wiring_with(&plane.control_plane, endpoints(&mock.base)).await;
    wiring.clock = clock.clone();
    (plane, wiring, listener, clock, mock)
}

async fn tick(wiring: &mut SupervisorWiring, clock: &TestClock, now: i64) {
    clock.0.store(now, Ordering::SeqCst);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        wiring.tick_all(now).await.unwrap();
        while let Some((key, result)) = wiring.tasks.join_next().await {
            assert!(result.is_ok(), "task {key} failed: {result:?}");
        }
    })
    .await
    .expect("bounded scheduling pass");
}

async fn ready(plane: &TestPlane, wiring: &mut SupervisorWiring, clock: &TestClock) {
    for step in 0..4 {
        tick(wiring, clock, NOW + step * 1000).await;
        if plane
            .control_plane
            .fleet_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .phase
            == "Ready"
        {
            return;
        }
    }
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    panic!("fleet did not reach Ready: {head:?}");
}

#[tokio::test]
async fn pending_fleet_becomes_ready_and_consumes_the_real_message_protocol() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    let before = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(before.phase, "Pending");
    assert_eq!(before.observed_revision, 0);
    assert_eq!(
        plane
            .control_plane
            .fleet_change_get("f1-put")
            .await
            .unwrap()
            .unwrap()
            .state,
        "Pending"
    );
    assert!(plane
        .control_plane
        .handoff_get(FLEET)
        .await
        .unwrap()
        .unwrap()
        .observed
        .is_none());

    ready(&plane, &mut wiring, &clock).await;
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(head.observed_revision, head.desired_revision);
    assert_eq!(
        plane
            .control_plane
            .fleet_change_get("f1-put")
            .await
            .unwrap()
            .unwrap()
            .state,
        "Succeeded"
    );
    let session = plane
        .control_plane
        .session_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.scale_set_id, 42);
    assert_eq!(session.auth_context.profile_key, KEY);
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 1);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);

    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 5_000).await;
    assert_eq!(
        plane.control_plane.demand_get(FLEET).await.unwrap(),
        Some(2)
    );
    assert_eq!(
        plane
            .control_plane
            .session_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .last_message_id,
        1
    );
    assert_eq!(listener.acks.load(Ordering::SeqCst), 1);
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 1);

    // Redelivery is durable and idempotent across scheduler/cache refreshes.
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 6_000).await;
    assert_eq!(listener.acks.load(Ordering::SeqCst), 1);
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 1);
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 1);
    assert_eq!(
        plane.control_plane.demand_get(FLEET).await.unwrap(),
        Some(2)
    );
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn existing_blocked_binding_recovers_from_lowercase_system_labels() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    *listener.label_type.lock().unwrap() = Some("customer".into());
    tick(&mut wiring, &clock, NOW).await;
    tick(&mut wiring, &clock, NOW + 1_000).await;
    let blocked = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(blocked.phase, "Degraded");
    assert_eq!(
        blocked.last_condition_reason.as_deref(),
        Some("OwnershipConflict")
    );
    assert_eq!(
        plane
            .control_plane
            .scale_set_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .state,
        "AccessBlocked"
    );
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 0);

    *listener.label_type.lock().unwrap() = Some("system".into());
    tick(&mut wiring, &clock, NOW + 2_000).await;
    assert_eq!(
        plane
            .control_plane
            .fleet_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .phase,
        "Ready"
    );
    assert_eq!(
        plane
            .control_plane
            .scale_set_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .scale_set_id,
        Some(42)
    );
    assert_eq!(
        plane
            .control_plane
            .fleet_change_get("f1-put")
            .await
            .unwrap()
            .unwrap()
            .state,
        "Succeeded"
    );
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    *listener.label_type.lock().unwrap() = Some("customer".into());
    tick(&mut wiring, &clock, NOW + 3_000).await;
    let drifted = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(drifted.phase, "Ready", "{drifted:?}");
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 1);
    tick(&mut wiring, &clock, NOW + 4_000).await;
    let recovered = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(recovered.phase, "Ready");
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 1);
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 1);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn expired_queue_reconnects_with_a_new_durable_session_epoch() {
    let (plane, mut wiring, listener, clock, _) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    let original = plane
        .control_plane
        .session_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    listener.expired.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 5_000).await;
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(head.phase, "Degraded");
    assert_eq!(
        head.last_condition_reason.as_deref(),
        Some("Unauthenticated")
    );
    // Retain the old handle until the exclusive replacement can safely close
    // it; the listener marks it disabled and reconnects after bounded backoff.
    assert_eq!(
        plane
            .control_plane
            .session_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .epoch,
        original.epoch
    );

    listener.expired.store(false, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 35_000).await;
    let current = plane
        .control_plane
        .session_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert!(current.epoch > original.epoch);
    assert_ne!(current.handle.session_id, original.handle.session_id);
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 2);
    assert_eq!(listener.session_deletes.load(Ordering::SeqCst), 1);
    assert_eq!(
        plane
            .control_plane
            .fleet_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .phase,
        "Ready"
    );
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn denied_queue_is_visible_as_degraded_and_recovers_after_retry() {
    let (plane, mut wiring, listener, clock, _) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    listener.denied.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 5_000).await;
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(head.phase, "Degraded");
    assert_eq!(
        head.last_condition_reason.as_deref(),
        Some("PermissionDenied")
    );
    listener.denied.store(false, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 35_000).await;
    tick(&mut wiring, &clock, NOW + 36_000).await;
    assert_eq!(
        plane
            .control_plane
            .fleet_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .phase,
        "Ready"
    );
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 1);
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn expired_ack_preserves_unacknowledged_work_until_session_replacement() {
    let (plane, mut wiring, listener, clock, _) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    let original = plane
        .control_plane
        .session_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    listener.expired_ack.store(true, Ordering::SeqCst);
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 5_000).await;
    let blocked = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(blocked.phase, "Degraded");
    assert_eq!(
        blocked.last_condition_reason.as_deref(),
        Some("Unauthenticated")
    );
    assert_eq!(
        plane
            .control_plane
            .session_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .last_message_id,
        0
    );
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 0);

    listener.expired_ack.store(false, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 35_000).await;
    let current = plane
        .control_plane
        .session_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert!(current.epoch > original.epoch);
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 36_000).await;
    assert_eq!(listener.acks.load(Ordering::SeqCst), 2);
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 1);
    assert_eq!(
        plane
            .control_plane
            .session_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .last_message_id,
        1
    );
    assert_eq!(
        plane.control_plane.demand_get(FLEET).await.unwrap(),
        Some(2)
    );
    wiring.tasks.shutdown().await;
}
