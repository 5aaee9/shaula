//! Multi-account GitHub authentication over the HTTP control plane
//! (spec 0011 §6): versioned publication, per-revision policy/binding
//! reads, version-guessing and mixed-member rejection, and the v2-head
//! refusal of legacy downgrades.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[path = "auth_v2_control_plane/fleet_admission.rs"]
mod fleet_admission;
use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::AccountKind;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthPromotion, AuthValidationSnapshot, ControlPlaneStore,
};
use shaula_store::registry_impl::SqliteControlPlane;
use tower::ServiceExt;

const V2_PUT_BODY: &str = r#"{
    "kind": "github_app",
    "schema_version": 2,
    "app_id": "4863460",
    "private_key": "-----BEGIN PRIVATE KEY-----\nZmFrZQ==\n-----END PRIVATE KEY-----",
    "target_policy": [
        {"kind": "organization", "owner": "Indexyz"},
        {"kind": "account_repositories", "account_kind": "user", "owner": "5aaee9"}
    ]
}"#;

const LEGACY_APP_PUT_BODY: &str = r#"{
    "kind": "github_app",
    "app_id": "4863460",
    "installation_id": 34,
    "private_key": "legacy-pem",
    "target_allowlist": [{"kind":"organization","owner":"Indexyz"}]
}"#;

fn frozen_bindings() -> Vec<AccountBinding> {
    vec![
        AccountBinding {
            account_id: 100,
            account_kind: AccountKind::Organization,
            login: "Indexyz".into(),
            installation_id: 11,
            repository_selection: RepositorySelection::All,
            validated_at_ms: 4,
        },
        AccountBinding {
            account_id: 200,
            account_kind: AccountKind::User,
            login: "5aaee9".into(),
            installation_id: 22,
            repository_selection: RepositorySelection::Selected,
            validated_at_ms: 4,
        },
    ]
}

/// Simulates the out-of-band validator: freezes bindings + a snapshot
/// consistent with the CURRENT live dependent set and promotes.
async fn promotion_now(store: &Arc<SqliteControlPlane>) -> AuthPromotion {
    let dependents = store.auth_live_dependents("shared-github").await.unwrap();
    let snapshot = AuthValidationSnapshot {
        candidate: ("shared-github".to_string(), 1),
        dependent_set: auth_dependent_set_fingerprint(&dependents),
        checked_fleets: dependents
            .iter()
            .map(|d| shaula_core::registry::AuthCheckedFleet {
                key: d.fleet_key.clone(),
                incarnation: d.incarnation.clone(),
                revision: d.revision,
                fence: d.fence,
            })
            .collect(),
        identities: Vec::new(),
    };
    AuthPromotion {
        bindings: frozen_bindings(),
        snapshot_json: serde_json::to_string(&snapshot).unwrap(),
    }
}

#[tokio::test]
async fn v2_publication_is_versioned_and_bindings_are_attributed() {
    let (app, store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/shared-github";
    let put = app
        .clone()
        .oneshot(authorized("PUT", uri, Some(V2_PUT_BODY.into())))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::ACCEPTED, "v2 create accepted");

    // Before validation the Candidate policy is shown as DESIRED, never
    // as an active effective configuration; no merged identity string.
    let view = get_json(&app, uri).await;
    assert_eq!(view["schema_version"], 2);
    assert_eq!(view["app_id"], "4863460");
    assert!(view.get("target_allowlist").is_none());
    assert!(view.get("identity").is_none());
    assert_eq!(
        view["desired"]["target_policy"].as_array().unwrap().len(),
        2
    );
    assert!(view.get("active").is_none());

    let outcome = store
        .auth_apply_validation_v2(
            "shared-github",
            1,
            true,
            None,
            5,
            Some(promotion_now(&store).await),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        shaula_core::registry::AuthPromotionOutcome::Promoted
    );

    // After promotion the ACTIVE policy and its bindings are attributed to
    // the exact revision.
    let view = get_json(&app, uri).await;
    assert_eq!(view["active"]["revision"], 1);
    let bindings = view["active"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0]["installation_id"], 11);
    assert_eq!(bindings[0]["login"], "Indexyz");
    assert_eq!(bindings[1]["repository_selection"], "selected");
    assert!(view.get("desired").is_none(), "candidate merged away");

    // Bindings are retrievable per exact revision through the store read
    // model — no secret material in any of it.
    let bindings = store.auth_bindings_get("shared-github", 1).await.unwrap();
    assert_eq!(bindings.len(), 2);
}

#[tokio::test]
async fn v2_head_refuses_legacy_downgrade_and_identity_change() {
    let (app, store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/shared-github";
    let put = app
        .clone()
        .oneshot(authorized("PUT", uri, Some(V2_PUT_BODY.into())))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::ACCEPTED);
    let etag = put.headers()["etag"].clone();
    store
        .auth_apply_validation_v2(
            "shared-github",
            1,
            true,
            None,
            5,
            Some(promotion_now(&store).await),
        )
        .await
        .unwrap();

    // An unsupported old request fails schema admission before any replay.
    let mut legacy = authorized("PUT", uri, Some(LEGACY_APP_PUT_BODY.into()));
    legacy.headers_mut().remove("if-none-match");
    legacy.headers_mut().insert("if-match", etag);
    assert_eq!(
        app.clone().oneshot(legacy).await.unwrap().status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // A different App id under the same key is an identity conflict.
    let get = app
        .clone()
        .oneshot(authorized("GET", uri, None))
        .await
        .unwrap();
    let current_etag = get.headers()["etag"].clone();
    let mut other_app = authorized("PUT", uri, Some(V2_PUT_BODY.replace("4863460", "9999999")));
    other_app.headers_mut().remove("if-none-match");
    other_app.headers_mut().insert("if-match", current_etag);
    assert_eq!(
        app.clone().oneshot(other_app).await.unwrap().status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn v2_rejects_mixed_members_and_version_guessing() {
    let (app, _store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/shared-github";
    let cases = [
        // schema_version 2 mixed with the legacy installation member.
        r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","installation_id":34,"private_key":"pem","target_policy":[{"kind":"organization","owner":"o"}]}"#,
        // schema_version 2 mixed with the legacy allowlist member.
        r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","private_key":"pem","target_allowlist":[{"kind":"organization","owner":"o"}],"target_policy":[{"kind":"organization","owner":"o"}]}"#,
        // target_policy WITHOUT schema_version: the version is never
        // guessed from the payload shape.
        r#"{"kind":"github_app","app_id":"4863460","private_key":"pem","target_policy":[{"kind":"organization","owner":"o"}]}"#,
        // schema_version 1 is the legacy format: no policy field.
        r#"{"kind":"github_app","schema_version":1,"app_id":"4863460","installation_id":34,"private_key":"pem","target_allowlist":[{"kind":"organization","owner":"o"}],"target_policy":[{"kind":"organization","owner":"o"}]}"#,
        // A non-positive-decimal App id.
        r#"{"kind":"github_app","schema_version":2,"app_id":"Iv1.abc","private_key":"pem","target_policy":[{"kind":"organization","owner":"o"}]}"#,
        // Duplicate selectors.
        r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","private_key":"pem","target_policy":[{"kind":"organization","owner":"o"},{"kind":"organization","owner":"O"}]}"#,
        // Unknown selector variant (wildcard-like) is not a selector.
        r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","private_key":"pem","target_policy":[{"kind":"glob","pattern":"*"}]}"#,
    ];
    for body in cases {
        let response = app
            .clone()
            .oneshot(authorized("PUT", uri, Some(body.into())))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "body must be rejected: {body}"
        );
    }
}

#[tokio::test]
async fn v2_same_key_policy_evolution_publishes_new_revision() {
    let (app, store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/shared-github";
    app.clone()
        .oneshot(authorized("PUT", uri, Some(V2_PUT_BODY.into())))
        .await
        .unwrap();
    store
        .auth_apply_validation_v2(
            "shared-github",
            1,
            true,
            None,
            5,
            Some(promotion_now(&store).await),
        )
        .await
        .unwrap();

    // Same App, one more selector: an explicit policy PUBLICATION on the
    // same key produces revision 2 — never a rejection.
    let get = app
        .clone()
        .oneshot(authorized("GET", uri, None))
        .await
        .unwrap();
    let etag = get.headers()["etag"].clone();
    let expanded = V2_PUT_BODY.replace(
        r#""target_policy": ["#,
        r#""target_policy": [
        {"kind": "organization", "owner": "second-org"},"#,
    );
    let mut put = authorized("PUT", uri, Some(expanded));
    put.headers_mut().remove("if-none-match");
    put.headers_mut().insert("if-match", etag);
    let response = app.clone().oneshot(put).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let view = get_json(&app, uri).await;
    assert_eq!(view["desiredRevision"], 2);
    // The expanded policy is staged as DESIRED while revision 1 stays
    // active — staged activation, never a merged view.
    assert_eq!(view["active"]["revision"], 1);
    assert_eq!(
        view["desired"]["target_policy"].as_array().unwrap().len(),
        3
    );
}
