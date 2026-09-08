//! Template/auth persistence tests.
#![allow(clippy::unwrap_used)]

use crate::store::Store;
pub(crate) async fn store() -> Store {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("p2.db");
    std::mem::forget(tmp);
    let s = Store::open(&path).await.unwrap();
    s.migrate().await.unwrap();
    s
}

#[tokio::test]
async fn auth_credential_plaintext_round_trip() {
    let store = store().await;
    let tx = store.begin().await.unwrap();
    store
        .auth_commit_revision(
            &tx,
            shaula_core::auth::AuthRevisionInsert {
                key: "prod-app".into(),
                incarnation: "inc-1".into(),
                revision: 1,
                kind: "pat".into(),
                app_id: None,
                installation_id: None,
                pat_principal: Some("octocat".into()),
                allowlist_json: r#"[{"kind":"organization","owner":"example-org"}]"#.into(),
                schema_version: 1,
                policy_json: None,
            },
            b"github_pat_SECRETBYTES",
            1,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    store
        .auth_apply_full("prod-app", 1, true, None, 2, None)
        .await
        .unwrap();

    let active = store
        .auth_revision_active("prod-app")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.credential_bytes, b"github_pat_SECRETBYTES");
    assert_eq!(active.kind, "pat");
    assert_eq!(active.pat_principal.as_deref(), Some("octocat"));

    // Rejected candidate leaves the previous active untouched.
    let tx = store.begin().await.unwrap();
    store
        .auth_commit_revision(
            &tx,
            shaula_core::auth::AuthRevisionInsert {
                key: "prod-app".into(),
                incarnation: "inc-1".into(),
                revision: 2,
                kind: "pat".into(),
                app_id: None,
                installation_id: None,
                pat_principal: Some("octocat".into()),
                allowlist_json: r#"[{"kind":"organization","owner":"example-org"}]"#.into(),
                schema_version: 1,
                policy_json: None,
            },
            b"github_pat_NEW",
            3,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    store
        .auth_apply_full("prod-app", 2, false, Some("Unauthenticated"), 4, None)
        .await
        .unwrap();

    let active = store
        .auth_revision_active("prod-app")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        active.revision, 1,
        "staged activation keeps the prior credential"
    );
    assert_eq!(active.credential_bytes, b"github_pat_SECRETBYTES");
}

#[tokio::test]
async fn generation_state_machine_enforced_at_persistence() {
    let store = store().await;
    store
        .generation_insert(shaula_core::registry::GenerationRecord {
            id: "g1".into(),
            fleet_key: "f1".into(),
            runner_name: "runner-1".into(),
            generation_name: "gen-1".into(),
            fleet_revision: 1,
            template_profile_key: "tpl".into(),
            template_revision: 2,
            template_artifact_digest: "sha256:art".into(),
            attestation_id: "att-1".into(),
            inputs_digest: "sha256:inputs".into(),
            state: shaula_core::lifecycle::GenerationState::CreatePending,
            github_runner_id: None,
            workspace_path: "/workspaces/g1".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();

    use shaula_core::lifecycle::GenerationState as S;
    store
        .generation_advance("g1", S::Creating, Some("ApplyStarting"), 2)
        .await
        .unwrap();
    store
        .generation_advance("g1", S::WaitingOnline, None, 3)
        .await
        .unwrap();
    store
        .generation_advance("g1", S::Idle, None, 4)
        .await
        .unwrap();

    // Illegal transition is rejected and leaves state untouched.
    let illegal = store.generation_advance("g1", S::Creating, None, 5).await;
    assert!(illegal.is_err());
    assert_eq!(
        store.generation_get("g1").await.unwrap().unwrap().state,
        "Idle"
    );

    // Legal terminal path still works.
    store
        .generation_advance("g1", S::Retiring, None, 6)
        .await
        .unwrap();
    store
        .generation_advance("g1", S::DestroyPending, None, 7)
        .await
        .unwrap();
    store
        .generation_advance("g1", S::Destroying, None, 8)
        .await
        .unwrap();
    store
        .generation_advance("g1", S::Destroyed, None, 9)
        .await
        .unwrap();
}

#[tokio::test]
async fn template_activation_requires_ready_state() {
    let store = store().await;
    let tx = store.begin().await.unwrap();
    store
        .template_commit_revision(
            &tx,
            shaula_core::template::TemplateRevisionInsert {
                key: "tpl".into(),
                incarnation: "inc-1".into(),
                revision: 1,
                artifact_digest: "sha256:a".into(),
                engine_ref: "terraform".into(),
                bindings_json: None,
                bindings_digest: Some("bd1_x".into()),
                fleet_input_policy_json: None,
            },
            1,
        )
        .await
        .unwrap();
    store
        .profile_change_insert(
            &tx,
            shaula_core::registry::ProfileChangeInsert {
                id: "pc-1".into(),
                resource_kind: "template_profile".into(),
                profile_key: "tpl".into(),
                revision: Some(1),
                kind: "publish".into(),
                now: 1,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Activation before Ready fails closed (inside the attestation tx).
    let tx = store.begin().await.unwrap();
    let premature = store.template_activate_tx(&tx, "tpl", 1, "att-1", 3).await;
    assert!(matches!(
        premature,
        Err(crate::store::StoreError::Conflict { .. })
    ));
    drop(tx);

    // Static validation only reaches Ready.
    store
        .template_revision_validated(shaula_core::template::TemplateValidationRecord {
            key: "tpl".into(),
            revision: 1,
            ready: true,
            platform: "kubernetes".into(),
            bindings_contract: "shaula.bindings.kubernetes/v1".into(),
            manifest_json: "{}".into(),
            lock_digest: "sha256:lock".into(),
            reason: None,
        })
        .await
        .unwrap();
    let revision = store
        .template_revision_get("tpl", 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(revision.state, "Ready");

    let tx = store.begin().await.unwrap();
    store
        .template_activate_tx(&tx, "tpl", 1, "att-1", 4)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let profile = store.template_profile_get("tpl").await.unwrap().unwrap();
    assert_eq!(profile.active_revision, Some(1));
    assert_eq!(profile.active_attestation_id.as_deref(), Some("att-1"));

    // One-time freeze (R5-09): a second activation — even for the same
    // Ready revision — conflicts and never replaces the frozen id.
    let tx = store.begin().await.unwrap();
    let second = store.template_activate_tx(&tx, "tpl", 1, "att-2", 5).await;
    assert!(matches!(
        second,
        Err(crate::store::StoreError::Conflict { .. })
    ));
    drop(tx);
    let profile = store.template_profile_get("tpl").await.unwrap().unwrap();
    assert_eq!(
        profile.active_attestation_id.as_deref(),
        Some("att-1"),
        "the frozen active attestation id must be immutable"
    );
}

#[tokio::test]
async fn auth_rotation_retargets_pinned_fleet_handoffs() {
    let store = store().await;
    use super::persistence::seed_auth_profile;
    for revision in 1..=2 {
        let tx = store.begin().await.unwrap();
        store
            .auth_commit_revision(
                &tx,
                shaula_core::auth::AuthRevisionInsert {
                    key: "prod-app".into(),
                    incarnation: "inc-1".into(),
                    revision,
                    kind: "pat".into(),
                    app_id: None,
                    installation_id: None,
                    pat_principal: Some("octocat".into()),
                    allowlist_json: r#"[{"kind":"organization","owner":"example-org"}]"#.into(),
                    schema_version: 1,
                    policy_json: None,
                },
                format!("cred-{revision}").as_bytes(),
                revision,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    seed_auth_profile(&store, "prod-app", "inc-1", 1).await;

    // A live fleet pinned to (prod-app, 1) with a matching handoff…
    let tx = store.begin().await.unwrap();
    store
        .fleet_commit_revision(
            &tx,
            shaula_core::fleet::FleetRevisionInsert {
                key: "fleet-live".into(),
                incarnation: "inc-l".into(),
                revision: 1,
                spec_json: "{}".into(),
                template: None,
                auth_desired: ("prod-app".into(), 1),
                inputs_digest: "d".into(),
                actor: "op".into(),
                now: 1,
            },
        )
        .await
        .unwrap();
    store
        .handoff_set_desired(&tx, "fleet-live", "prod-app", 1)
        .await
        .unwrap();
    // …and a decommissioning fleet whose handoff is cleanup-only.
    store
        .fleet_commit_revision(
            &tx,
            shaula_core::fleet::FleetRevisionInsert {
                key: "fleet-dying".into(),
                incarnation: "inc-d".into(),
                revision: 1,
                spec_json: "{}".into(),
                template: None,
                auth_desired: ("prod-app".into(), 1),
                inputs_digest: "d".into(),
                actor: "op".into(),
                now: 1,
            },
        )
        .await
        .unwrap();
    store
        .handoff_set_desired(&tx, "fleet-dying", "prod-app", 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let tx = store.begin().await.unwrap();
    store
        .handoff_set_cleanup_only(&tx, "fleet-dying")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Rotating the active revision to 2 must retarget ONLY the live fleet,
    // in the same transaction as the head advance.
    store
        .auth_apply_full("prod-app", 2, true, None, 9, None)
        .await
        .unwrap();

    let live = store.handoff_get("fleet-live").await.unwrap().unwrap();
    assert_eq!(live.desired_profile_key, "prod-app");
    assert_eq!(live.desired_revision, 2);
    assert_eq!(live.state, "Pending");

    // Cleanup-only handoffs ARE retargeted: the decommissioning fleet
    // may need the fresh credential to finish cleanup (spec 0005
    // cleanup handoff), while the flag keeps gating ordinary effects.
    let dying = store.handoff_get("fleet-dying").await.unwrap().unwrap();
    assert_eq!(dying.desired_revision, 2);
    assert!(dying.cleanup_only);
}
