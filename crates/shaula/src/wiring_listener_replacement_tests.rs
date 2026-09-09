//! Fleet revision replacement must settle its renewed auth handoff before effects.

use super::*;

async fn commit_same_auth_replacement(plane: &TestPlane) {
    let store = &plane.control_plane;
    let initial = store.fleet_get(FLEET).await.unwrap().unwrap();
    let original_revision = store.fleet_revision_latest(FLEET).await.unwrap().unwrap();
    let mut spec: serde_json::Value = serde_json::from_str(&original_revision.spec_json).unwrap();
    spec["template_profile_ref"]["revision"] = serde_json::json!(2);
    // Use the real SQLite commit path: a template selection changes the Fleet
    // fence and renews Pending handoff intent even though the auth ref is equal.
    store
        .commit_fleet_mutation(MutationFacts {
            resource_kind: "fleet",
            resource_key: FLEET.into(),
            incarnation: initial.incarnation.clone(),
            revision: 2,
            spec_json: serde_json::to_string(&spec).unwrap(),
            template: None,
            auth_desired: Some(original_revision.auth_desired.clone()),
            inputs_digest: original_revision.inputs_digest,
            actor: "op".into(),
            now: NOW + 5_000,
            change: ChangeView {
                id: "f1-template-replacement".into(),
                resource_kind: "fleet".into(),
                resource_key: FLEET.into(),
                revision: 2,
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
}

#[tokio::test]
async fn same_auth_fleet_revision_reproves_handoff_before_replacing_listener_session() {
    let (plane, mut wiring, listener, clock, mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    let store = &plane.control_plane;
    let initial = store.fleet_get(FLEET).await.unwrap().unwrap();
    let original_session = store.session_get(FLEET).await.unwrap().unwrap();
    let original_binding = store.scale_set_get(FLEET).await.unwrap().unwrap();
    commit_same_auth_replacement(&plane).await;

    let pending = store.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(pending.desired_revision, 2);
    assert!(pending.mutation_fence > initial.mutation_fence);
    let handoff = store.handoff_get(FLEET).await.unwrap().unwrap();
    assert_eq!(handoff.state, "Pending");
    assert_eq!(handoff.observed.as_ref(), Some(&handoff.desired));
    let proof_reads = mock.installation_reads.lock().unwrap().len();
    let sessions = listener.session_creates.load(Ordering::SeqCst);

    // The first pass must do a read-only proof and acknowledge the NEW fence;
    // equal auth references must not skip straight to a session effect.
    tick(&mut wiring, &clock, NOW + 6_000).await;
    let handoff = store.handoff_get(FLEET).await.unwrap().unwrap();
    assert_eq!(
        handoff.state, "Observed",
        "same-ref Pending handoff must settle"
    );
    assert_eq!(
        store
            .fleet_auth_context_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .state,
        "Observed"
    );
    assert!(mock.installation_reads.lock().unwrap().len() > proof_reads);
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), sessions);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);

    tick(&mut wiring, &clock, NOW + 7_000).await;
    let current = store.fleet_get(FLEET).await.unwrap().unwrap();
    assert_eq!(current.phase, "Ready", "{current:?}");
    assert_eq!(current.observed_revision, 2);
    assert_eq!(
        store
            .fleet_change_get("f1-template-replacement")
            .await
            .unwrap()
            .unwrap()
            .state,
        "Succeeded"
    );
    let session = store.session_get(FLEET).await.unwrap().unwrap();
    assert!(session.epoch > original_session.epoch);
    assert_ne!(
        session.handle.session_id,
        original_session.handle.session_id
    );
    assert_eq!(session.scale_set_id, original_session.scale_set_id);
    assert_eq!(
        listener.session_creates.load(Ordering::SeqCst),
        sessions + 1
    );
    assert_eq!(listener.session_deletes.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .scale_set_get(FLEET)
            .await
            .unwrap()
            .unwrap()
            .scale_set_id,
        original_binding.scale_set_id
    );
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);

    // Let the Ready phase refresh the cache, then verify settled reconciliation
    // does not re-run a handoff proof or manufacture another session.
    tick(&mut wiring, &clock, NOW + 8_000).await;
    let settled_reads = mock.installation_reads.lock().unwrap().len();
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 9_000).await;
    assert_eq!(mock.installation_reads.lock().unwrap().len(), settled_reads);
    assert_eq!(
        listener.session_creates.load(Ordering::SeqCst),
        sessions + 1
    );
    assert_eq!(listener.session_deletes.load(Ordering::SeqCst), 1);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    assert_eq!(listener.acks.load(Ordering::SeqCst), 1);
    assert_eq!(listener.acquisitions.load(Ordering::SeqCst), 1);
    assert_eq!(store.demand_get(FLEET).await.unwrap(), Some(2));
    assert_eq!(
        store.fleet_get(FLEET).await.unwrap().unwrap().phase,
        "Ready"
    );
    wiring.tasks.shutdown().await;
}

#[tokio::test]
async fn decommission_after_same_auth_replacement_accepts_only_the_current_handoff_fence() {
    use shaula_core::registry::{AuthHandoffExpectation, FleetContextAck, FleetRuntimeGuard};

    let (plane, mut wiring, listener, clock, mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    commit_same_auth_replacement(&plane).await;
    let store = &plane.control_plane;
    let pending = store.fleet_get(FLEET).await.unwrap().unwrap();
    let context = store.fleet_auth_context_get(FLEET).await.unwrap().unwrap();
    let handoff = store.handoff_get(FLEET).await.unwrap().unwrap();
    assert_eq!(handoff.state, "Pending");
    let sessions = listener.session_creates.load(Ordering::SeqCst);
    store
        .commit_decommission(MutationFacts {
            resource_kind: "fleet",
            resource_key: FLEET.into(),
            incarnation: pending.incarnation.clone(),
            revision: pending.desired_revision + 1,
            spec_json: "{}".into(),
            template: None,
            auth_desired: None,
            inputs_digest: "decommission".into(),
            actor: "op".into(),
            now: NOW + 6_000,
            change: ChangeView {
                id: "f1-delete-after-replacement".into(),
                resource_kind: "fleet".into(),
                resource_key: FLEET.into(),
                revision: pending.desired_revision + 1,
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
    let deleting = store.fleet_get(FLEET).await.unwrap().unwrap();
    assert!(deleting.deletion_marker);
    assert!(deleting.mutation_fence > pending.mutation_fence);
    let cleanup = store.handoff_get(FLEET).await.unwrap().unwrap();
    assert!(cleanup.cleanup_only);
    assert_eq!(cleanup.state, "Pending");
    assert_eq!(cleanup.desired, handoff.desired);
    let preserved = store.fleet_auth_context_get(FLEET).await.unwrap().unwrap();
    assert_eq!(preserved.state, context.state);
    assert_eq!(preserved.desired_context_json, context.desired_context_json);
    assert_eq!(
        preserved.observed_context_json,
        context.observed_context_json
    );

    // This boundary test uses the already verified identity as the result of
    // a fresh read-only proof. It exercises the real SQLite ack CAS directly:
    // a proof started before DELETE is stale, while a new cleanup proof settles.
    for (mutation_fence, expected) in [
        (pending.mutation_fence, FleetContextAck::Stale),
        (deleting.mutation_fence, FleetContextAck::Acknowledged),
    ] {
        assert_eq!(
            store
                .handoff_acknowledge(
                    FLEET,
                    &handoff.desired.0,
                    handoff.desired.1,
                    context.observed_context_json.as_deref(),
                    &AuthHandoffExpectation {
                        mutation_fence,
                        desired_context_json: context.desired_context_json.clone(),
                    },
                )
                .await
                .unwrap(),
            expected
        );
    }
    let settled = store.handoff_get(FLEET).await.unwrap().unwrap();
    assert!(settled.cleanup_only);
    assert_eq!(settled.state, "Observed");
    let auth_context =
        serde_json::from_str(context.observed_context_json.as_deref().unwrap()).unwrap();
    assert!(!store
        .session_authorize(FLEET, &FleetRuntimeGuard::from(&deleting), &auth_context)
        .await
        .unwrap());
    assert_eq!(listener.session_creates.load(Ordering::SeqCst), sessions);
    assert_eq!(mock.scale_set_creates.load(Ordering::SeqCst), 0);
    wiring.tasks.shutdown().await;
}
