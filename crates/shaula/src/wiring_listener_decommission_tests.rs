//! Spec 0002 §8: a deletion-marked fleet completes once every owned
//! Generation is terminal. Closing the listener session is best effort, so an
//! unreachable target (e.g. a transferred repository) must not hold the
//! tombstone forever.

use super::*;

#[tokio::test]
async fn decommission_tombstones_even_when_the_session_delete_fails() {
    let (plane, mut wiring, listener, clock, _mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    let store = &plane.control_plane;
    let head = store.fleet_get(FLEET).await.unwrap().unwrap();
    listener
        .session_delete_unavailable
        .store(true, Ordering::SeqCst);
    store
        .commit_decommission(MutationFacts {
            idempotency_operation: "v1:DELETE",
            authentication: Default::default(),
            resource_kind: "fleet",
            resource_key: FLEET.into(),
            incarnation: head.incarnation.clone(),
            revision: head.desired_revision + 1,
            spec_json: "{}".into(),
            template_pool: Vec::new(),
            template_pool_ref: None,
            template: None,
            auth_desired: None,
            inputs_digest: "decommission".into(),
            actor: "op".into(),
            now: NOW + 5_000,
            change: ChangeView {
                id: "f1-decommission".into(),
                resource_kind: "fleet".into(),
                resource_key: FLEET.into(),
                revision: head.desired_revision + 1,
                kind: "Decommission".into(),
                state: "Pending".into(),
                reason: None,
            },
            outbox_topic: "fleet.change".into(),
            outbox_payload: "{}".into(),
            idempotency: None,
        })
        .await
        .unwrap()
        .unwrap();

    for step in 0..4 {
        tick(&mut wiring, &clock, NOW + 6_000 + step * 1000).await;
        if store.fleet_get(FLEET).await.unwrap().unwrap().tombstone {
            break;
        }
    }
    assert!(
        listener.session_deletes.load(Ordering::SeqCst) > 0,
        "the listener must still attempt to close its session"
    );
    let head = store.fleet_get(FLEET).await.unwrap().unwrap();
    assert!(head.tombstone, "{head:?}");
    assert_eq!(head.phase, "Decommissioned");
    let change = store
        .fleet_change_get("f1-decommission")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(change.state, "Succeeded", "{change:?}");
    wiring.tasks.shutdown().await;
}
