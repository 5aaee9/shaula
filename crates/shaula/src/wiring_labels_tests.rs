//! Mutable labels through real SQLite, scheduling, supervisor and HTTP adapter.

use super::*;
use serde_json::json;

pub(super) async fn commit_labels(plane: &TestPlane, labels: &[&str], now: i64) -> i64 {
    let store = &plane.control_plane;
    let head = store.fleet_get(FLEET).await.unwrap().unwrap();
    let previous = store.fleet_revision_latest(FLEET).await.unwrap().unwrap();
    let mut spec: serde_json::Value = serde_json::from_str(&previous.spec_json).unwrap();
    spec["github"]["labels"] = json!(labels);
    let revision = head.desired_revision + 1;
    store
        .commit_fleet_mutation(MutationFacts {
            resource_kind: "fleet",
            resource_key: FLEET.into(),
            incarnation: head.incarnation,
            revision,
            spec_json: serde_json::to_string(&spec).unwrap(),
            template: None,
            auth_desired: Some(previous.auth_desired),
            inputs_digest: previous.inputs_digest,
            actor: "op".into(),
            now,
            change: ChangeView {
                id: format!("f1-labels-{revision}"),
                resource_kind: "fleet".into(),
                resource_key: FLEET.into(),
                revision,
                kind: "Put".into(),
                state: "Pending".into(),
                reason: None,
            },
            outbox_topic: "fleet.reconcile".into(),
            outbox_payload: "{}".into(),
            idempotency: None,
        })
        .await
        .unwrap()
        .unwrap();
    revision
}

#[tokio::test]
async fn owned_labels_add_remove_and_clear_converge_on_the_same_scale_set() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    let original = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    for (index, labels) in [&["linux", "arm64"][..], &["linux"][..], &[][..]]
        .into_iter()
        .enumerate()
    {
        let now = NOW + 5_000 + index as i64 * 3_000;
        let revision = commit_labels(&plane, labels, now).await;
        tick(&mut wiring, &clock, now + 1_000).await;
        tick(&mut wiring, &clock, now + 2_000).await;
        let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
        assert_eq!(head.phase, "Ready", "{head:?}");
        assert_eq!(head.incarnation, original.incarnation);
        assert_eq!(head.observed_revision, revision);
        assert!(head.mutation_fence > original.mutation_fence);
        let owned = plane
            .control_plane
            .scale_set_get(FLEET)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owned.scale_set_id, Some(42));
        assert_eq!(owned.owned_scale_set_id, Some(42));
        assert_eq!(owned.state, "Adopted");
        assert_eq!(listener.label_updates.load(Ordering::SeqCst), index + 1);
        let expected = if labels.is_empty() {
            json!([{"name":"shaula-x64","type":"System"}])
        } else {
            json!(labels
                .iter()
                .map(|name| json!({"name":name,"type":"System"}))
                .collect::<Vec<_>>())
        };
        assert_eq!(listener.scale_set()["labels"], expected);
        assert_eq!(
            plane
                .control_plane
                .fleet_change_get(&format!("f1-labels-{revision}"))
                .await
                .unwrap()
                .unwrap()
                .state,
            "Succeeded"
        );
        assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    }
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn put_success_without_changed_readback_stays_pending_and_blocks_acquisition() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    listener.ignore_label_updates.store(true, Ordering::SeqCst);
    commit_labels(&plane, &["linux"], NOW + 5_000).await;
    tick(&mut wiring, &clock, NOW + 6_000).await;
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 7_000).await;
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_ne!(head.phase, "Ready");
    assert_eq!(
        head.last_condition_reason.as_deref(),
        Some("ScaleSetLabelsPending")
    );
    let owned = plane
        .control_plane
        .scale_set_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owned.state, "LabelsUpdating");
    assert_eq!(owned.owned_scale_set_id, Some(42));
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 1);
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 0);
    assert_ne!(
        plane
            .control_plane
            .fleet_change_get("f1-labels-2")
            .await
            .unwrap()
            .unwrap()
            .state,
        "Succeeded"
    );
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn malformed_applied_put_response_recovers_from_readback_after_wiring_restart() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    listener
        .malformed_label_response
        .store(true, Ordering::SeqCst);
    commit_labels(&plane, &["linux"], NOW + 5_000).await;
    tick(&mut wiring, &clock, NOW + 6_000).await;
    tick(&mut wiring, &clock, NOW + 7_000).await;
    let blocked = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_ne!(blocked.phase, "Ready");
    assert_eq!(
        blocked.last_condition_reason.as_deref(),
        Some("AccessVerificationFailed")
    );
    let owned = plane
        .control_plane
        .scale_set_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owned.state, "AccessBlocked");
    assert_eq!(owned.owned_scale_set_id, Some(42));
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 1);
    wiring.tasks.shutdown().await;
    drop(wiring);

    let mut restarted = wiring_with(&plane.control_plane, endpoints(&mock.base)).await;
    restarted.clock = clock.clone();
    tick(&mut restarted, &clock, NOW + 8_000).await;
    let recovered = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(recovered.phase, "Ready", "{recovered:?}");
    assert_eq!(recovered.observed_revision, 2);
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 1);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    restarted.tasks.shutdown().await;
}

#[tokio::test]
async fn same_name_unowned_labels_conflict_never_sends_a_put() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    *listener.label_values.lock().unwrap() = Some(json!([{"name":"foreign","type":"User"}]));
    tick(&mut wiring, &clock, NOW).await;
    tick(&mut wiring, &clock, NOW + 1_000).await;
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(
        head.last_condition_reason.as_deref(),
        Some("OwnershipConflict")
    );
    let candidate = plane
        .control_plane
        .scale_set_get(FLEET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(candidate.scale_set_id, Some(42));
    assert_eq!(candidate.owned_scale_set_id, None);
    commit_labels(&plane, &["linux"], NOW + 5_000).await;
    tick(&mut wiring, &clock, NOW + 6_000).await;
    tick(&mut wiring, &clock, NOW + 7_000).await;
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 0);
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), 0);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn owned_labels_mismatch_with_unknown_runner_inventory_never_sends_a_put() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    listener.unknown_runner.store(true, Ordering::SeqCst);
    commit_labels(&plane, &["linux"], NOW + 5_000).await;
    tick(&mut wiring, &clock, NOW + 6_000).await;
    tick(&mut wiring, &clock, NOW + 7_000).await;
    let head = plane.control_plane.fleet_get(FLEET).await.unwrap().unwrap();
    assert_ne!(head.phase, "Ready");
    assert_eq!(
        head.last_condition_reason.as_deref(),
        Some("UnknownRemoteRunner")
    );
    assert_eq!(listener.label_updates.load(Ordering::SeqCst), 0);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    wiring.tasks.shutdown().await;
}
