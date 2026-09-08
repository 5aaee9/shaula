//! Multi-account auth upgrade-path HTTP tests (R7/R10): legacy
//! idempotency replay continuity, strict field presence and the
//! revision-attributed staged-upgrade GET shape.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use axum::http::StatusCode;
use common::*;
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

const LEGACY_APP_PUT_BODY: &str = r#"{
    "kind": "github_app",
    "app_id": "4863460",
    "installation_id": 34,
    "private_key": "legacy-pem",
    "target_allowlist": [{"kind":"organization","owner":"Indexyz"}]
}"#;

#[tokio::test]
async fn legacy_idempotent_replay_survives_the_v2_upgrade_encoding() {
    use shaula_core::auth::request_hash_parts;
    use shaula_core::auth::TargetAllowlist;
    use shaula_core::github::GitHubTarget;
    use shaula_core::registry::IdempotencyLookup;
    let (app, store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/prod-app";
    let mut put = put_with_idempotency(uri, "put-legacy", AUTH_PUT_BODY.to_string());
    let _ = &mut put;
    let response = app.clone().oneshot(put).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let etag = response.headers()["etag"].clone();
    let first_body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();

    // The BASELINE encoder: five unprefixed members over the parsed
    // allowlist. The stored hash must equal it.
    let allowlist_json = serde_json::to_string(
        &TargetAllowlist::new(vec![GitHubTarget::organization("example-org").unwrap()]).unwrap(),
    )
    .unwrap();
    let baseline_body = format!("pat||0|octocat|{allowlist_json}");
    let baseline_hash = request_hash_parts(&[
        b"github_auth_profile",
        "prod-app".as_bytes(),
        "put-legacy".as_bytes(),
        baseline_body.as_bytes(),
    ]);
    match store
        .idempotency_find(
            "github_auth_profile",
            "prod-app",
            "put-legacy",
            &baseline_hash,
        )
        .await
        .unwrap()
    {
        IdempotencyLookup::Replay(body) => {
            let original: serde_json::Value = serde_json::from_slice(&first_body).unwrap();
            let replayed: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(
                replayed["change"]["id"], original["changeId"],
                "exact replay of the original mutation"
            );
            assert_eq!(replayed["change"]["revision"], 1);
        }
        other => panic!("baseline encoding must replay, got {other:?}"),
    }
    // A version-prefixed hash is a DIFFERENT request: conflict, not replay.
    assert!(matches!(
        store
            .idempotency_find(
                "github_auth_profile",
                "prod-app",
                "put-legacy",
                &request_hash_parts(&[
                    b"github_auth_profile",
                    "prod-app".as_bytes(),
                    "put-legacy".as_bytes(),
                    format!("1|{baseline_body}").as_bytes()
                ])
            )
            .await
            .unwrap(),
        IdempotencyLookup::Conflict
    ));

    // Same key + changed secret: genuine conflict, never a silent replay.
    let changed = AUTH_PUT_BODY.replace("github_pat_test_token_bytes", "CHANGED");
    let mut retry = put_with_idempotency(uri, "put-legacy", changed);
    retry.headers_mut().remove("if-none-match");
    retry.headers_mut().insert("if-match", etag.clone());
    let conflict = app.clone().oneshot(retry).await.unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
}

/// R10: strict field presence — forbidden members are rejected regardless
/// of value (`target_allowlist: []` in v2) and explicit nulls never parse.
#[tokio::test]
async fn v2_rejects_empty_forbidden_members_and_explicit_nulls() {
    let (app, _store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/shared-github";
    let cases = [
        // Forbidden legacy member present but EMPTY is still forbidden.
        r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","private_key":"pem","target_allowlist":[],"target_policy":[{"kind":"organization","owner":"o"}]}"#,
        // Forbidden legacy member present as NULL is still forbidden.
        r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","private_key":"pem","target_allowlist":null,"target_policy":[{"kind":"organization","owner":"o"}]}"#,
        // Explicit null on a REQUIRED v2 member rejects.
        r#"{"kind":"github_app","schema_version":2,"app_id":null,"private_key":"pem","target_policy":[{"kind":"organization","owner":"o"}]}"#,
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

    // F10: an IRRELEVANT explicit null in a LEGACY body behaves as absent
    // (baseline semantics preserved) — the PUT is accepted.
    let legacy_nulls = r#"{"kind":"github_app","app_id":"4863460","installation_id":null,"private_key":"pem","pat_principal":null,"target_allowlist":[{"kind":"organization","owner":"example-org"}]}"#;
    let response = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/github-auth-profiles/prod-app",
            Some(legacy_nulls.into()),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "null legacy members behave as absent"
    );
}

/// R10: during a staged legacy→v2 upgrade the still-effective legacy
/// authorization stays visible (top-level identity/target_allowlist AND
/// the attributed active object) alongside the Candidate policy.
#[tokio::test]
async fn staged_upgrade_keeps_effective_legacy_authorization_visible() {
    let (app, store, _) = build_app_with_scan().await;
    let uri = "/api/v1/github-auth-profiles/prod-app";
    let put = app
        .clone()
        .oneshot(authorized("PUT", uri, Some(LEGACY_APP_PUT_BODY.into())))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::ACCEPTED);
    // Activate the legacy revision (out-of-band validator, legacy path).
    store
        .auth_apply_validation_v2("prod-app", 1, true, None, 5, None)
        .await
        .unwrap();

    // Pure legacy GET: the exact baseline shape.
    let view = get_json(&app, uri).await;
    assert_eq!(view["identity"], "app/4863460/installation/34");
    assert!(view.get("schema_version").is_none());

    // Stage the v2 upgrade on the same key.
    let v2_body = r#"{
        "kind": "github_app",
        "schema_version": 2,
        "app_id": "4863460",
        "private_key": "pem",
        "target_policy": [{"kind": "organization", "owner": "Indexyz"}]
    }"#;
    let get = app
        .clone()
        .oneshot(authorized("GET", uri, None))
        .await
        .unwrap();
    let etag = get.headers()["etag"].clone();
    let mut upgrade = authorized("PUT", uri, Some(v2_body.into()));
    upgrade.headers_mut().remove("if-none-match");
    upgrade.headers_mut().insert("if-match", etag);
    let upgrade_response = app.clone().oneshot(upgrade).await.unwrap();
    assert_eq!(upgrade_response.status(), StatusCode::ACCEPTED);

    // During Pending upgrade: legacy authorization still visible at the
    // top level AND the candidate policy attributed to the desired head.
    let view = get_json(&app, uri).await;
    assert_eq!(view["identity"], "app/4863460/installation/34");
    assert_eq!(view["target_allowlist"].as_array().unwrap().len(), 1);
    assert_eq!(view["schema_version"], 2);
    assert_eq!(view["active"]["revision"], 1);
    assert_eq!(view["active"]["schema_version"], 1);
    assert_eq!(view["desired"]["revision"], 2);
    assert_eq!(
        view["desired"]["target_policy"].as_array().unwrap().len(),
        1
    );
}
