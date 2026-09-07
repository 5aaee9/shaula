//! Auth-rotation → handoff retarget persistence tests (spec 0002 §299),
//! split to keep files within 400 lines (AGENTS.md).
#![allow(clippy::unwrap_used)]

use super::profiles::store;

#[tokio::test]
async fn auth_rotation_retarget_ignores_fleet_revision_collision() {
    // The fleet's own revision counter and the auth profile's revision
    // counter are independent sequences: a fleet whose LATEST fleet
    // revision numerically equals the promoted auth revision must STILL be
    // retargeted (regression guard for comparing the two ID spaces).
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
            },
            b"cred-1",
            1,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    store
        .auth_apply_validation("prod-app", 1, true, None, 8)
        .await
        .unwrap();

    // Fleet pinned to (prod-app, 1) at fleet revision 2 — numerically equal
    // to the auth revision about to be promoted.
    for fleet_revision in 1..=2 {
        let tx = store.begin().await.unwrap();
        store
            .fleet_commit_revision(
                &tx,
                shaula_core::fleet::FleetRevisionInsert {
                    key: "fleet-collide".into(),
                    incarnation: "inc-c".into(),
                    revision: fleet_revision,
                    spec_json: "{}".into(),
                    template: None,
                    auth_desired: ("prod-app".into(), 1),
                    inputs_digest: "d".into(),
                    actor: "op".into(),
                    now: fleet_revision,
                },
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let tx = store.begin().await.unwrap();
    store
        .handoff_set_desired(&tx, "fleet-collide", "prod-app", 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Candidate auth revision 2 advances the desired head; promoting it
    // (== fleet's latest revision number) must still retarget the handoff
    // to (prod-app, 2).
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
            },
            b"cred-2",
            9,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    store
        .auth_apply_validation("prod-app", 2, true, None, 10)
        .await
        .unwrap();

    let handoff = store.handoff_get("fleet-collide").await.unwrap().unwrap();
    assert_eq!(handoff.desired_profile_key, "prod-app");
    assert_eq!(handoff.desired_revision, 2);
}

#[tokio::test]
async fn auth_rotation_never_undoes_cross_profile_switch() {
    // A fleet that explicitly switched from profile "old-app" to
    // "other-app" (cross-profile replacement at zero occupancy) must NOT
    // be dragged back when the OLD profile rotates.
    let store = store().await;

    // Activate "other-app" revision 1 so the fleet can pin it.
    let tx = store.begin().await.unwrap();
    for (key, revision) in [("old-app", 1), ("other-app", 1)] {
        store
            .auth_commit_revision(
                &tx,
                shaula_core::auth::AuthRevisionInsert {
                    key: key.into(),
                    incarnation: "inc-1".into(),
                    revision,
                    kind: "pat".into(),
                    app_id: None,
                    installation_id: None,
                    pat_principal: Some("octocat".into()),
                    allowlist_json: r#"[{"kind":"organization","owner":"example-org"}]"#.into(),
                },
                b"cred",
                1,
            )
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    store
        .auth_apply_validation("old-app", 1, true, None, 6)
        .await
        .unwrap();
    store
        .auth_apply_validation("other-app", 1, true, None, 8)
        .await
        .unwrap();

    // Fleet history: revision 1 pinned OLD (simulating a prior state),
    // latest revision 2 pins OTHER — the current dependency.
    for (fleet_revision, profile) in [(1, "old-app"), (2, "other-app")] {
        let tx = store.begin().await.unwrap();
        store
            .fleet_commit_revision(
                &tx,
                shaula_core::fleet::FleetRevisionInsert {
                    key: "fleet-switched".into(),
                    incarnation: "inc-f".into(),
                    revision: fleet_revision,
                    spec_json: "{}".into(),
                    template: None,
                    auth_desired: (profile.into(), 1),
                    inputs_digest: "d".into(),
                    actor: "op".into(),
                    now: fleet_revision,
                },
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let tx = store.begin().await.unwrap();
    store
        .handoff_set_desired(&tx, "fleet-switched", "other-app", 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Rotating OLD to revision 2 selects fleets by CURRENT dependency —
    // the handoff pins other-app, so nothing may change.
    let tx = store.begin().await.unwrap();
    store
        .auth_commit_revision(
            &tx,
            shaula_core::auth::AuthRevisionInsert {
                key: "old-app".into(),
                incarnation: "inc-1".into(),
                revision: 2,
                kind: "pat".into(),
                app_id: None,
                installation_id: None,
                pat_principal: Some("octocat".into()),
                allowlist_json: r#"[{"kind":"organization","owner":"example-org"}]"#.into(),
            },
            b"cred-old-2",
            9,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    store
        .auth_apply_validation("old-app", 2, true, None, 10)
        .await
        .unwrap();

    let handoff = store.handoff_get("fleet-switched").await.unwrap().unwrap();
    assert_eq!(
        handoff.desired_profile_key, "other-app",
        "the operator's cross-profile switch must stand"
    );
    assert_eq!(handoff.desired_revision, 1);
}
