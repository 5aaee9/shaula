//! HTTP read contracts against the real daemon and SQLite repositories.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::{http::StatusCode, Router};
use common::{authorized, build_app_with_scan, get_json};
use shaula_core::auth_context::{AccountBinding, RepositorySelection, ResolvedAuthContext};
use shaula_core::auth_policy::AccountKind;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthExecutionStore, AuthHandoffExpectation, AuthPromotion,
    AuthRouteObservation, AuthValidationSnapshot, ChangeView, ControlPlaneStore, FleetContextAck,
    MutationFacts,
};
use shaula_store::registry_impl::SqliteControlPlane;
use tower::ServiceExt;

const NOW: i64 = 1_800_000_000_000;
const URI: &str = "/api/v1/github-auth-profiles/shared-github";
const V2: &str = r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","private_key":"fixture-pem","target_policy":[{"kind":"organization","owner":"Indexyz"},{"kind":"account_repositories","account_kind":"user","owner":"5aaee9"}]}"#;

async fn replace(app: &Router, body: String) {
    let response = app
        .clone()
        .oneshot(authorized("GET", URI, None))
        .await
        .unwrap();
    let mut put = authorized("PUT", URI, Some(body));
    put.headers_mut().remove("if-none-match");
    put.headers_mut()
        .insert("if-match", response.headers()["etag"].clone());
    assert_eq!(
        app.clone().oneshot(put).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
}

async fn promote(store: &SqliteControlPlane) {
    let bindings = [
        (100, AccountKind::Organization, "Indexyz", 11, NOW - 60_000),
        (200, AccountKind::User, "5aaee9", 22, NOW),
    ]
    .into_iter()
    .map(|(id, kind, login, install, checked)| AccountBinding {
        account_id: id,
        account_kind: kind,
        login: login.into(),
        installation_id: install,
        repository_selection: RepositorySelection::All,
        validated_at_ms: checked,
    })
    .collect();
    let snapshot = AuthValidationSnapshot {
        candidate: ("shared-github".into(), 1),
        dependent_set: auth_dependent_set_fingerprint(&[]),
        checked_fleets: vec![],
        identities: vec![],
    };
    store
        .auth_apply_validation_v2(
            "shared-github",
            1,
            true,
            None,
            NOW,
            Some(AuthPromotion {
                bindings,
                snapshot_json: serde_json::to_string(&snapshot).unwrap(),
            }),
        )
        .await
        .unwrap();
}

async fn seed_fleet(store: &SqliteControlPlane, key: &str) {
    let mut spec: serde_json::Value = serde_json::from_str(common::FLEET_BODY).unwrap();
    spec["github"]["target"] =
        serde_json::json!({"kind":"repository","owner":"5aaee9","repository":"repo"});
    spec["github"]["auth_profile_ref"] = serde_json::json!("shared-github");
    store
        .commit_fleet_mutation(MutationFacts {
            resource_kind: "fleet",
            resource_key: key.into(),
            incarnation: format!("{key}-inc"),
            revision: 1,
            spec_json: spec.to_string(),
            template: None,
            auth_desired: Some(("shared-github".into(), 1)),
            inputs_digest: "fixture".into(),
            actor: "test".into(),
            now: NOW,
            change: ChangeView {
                id: format!("change-{key}"),
                resource_kind: "fleet".into(),
                resource_key: key.into(),
                revision: 1,
                kind: "Create".into(),
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
}

#[tokio::test]
async fn real_legacy_wire_and_rejected_upgrade_keep_exact_revision_authority() {
    let (app, store, _) = build_app_with_scan().await;
    let legacy = r#"{"kind":"github_app","app_id":"Iv23legacy","installation_id":34,"private_key":"fixture-pem","target_allowlist":[{"kind":"organization","owner":"Indexyz"}]}"#;
    assert_eq!(
        app.clone()
            .oneshot(authorized("PUT", URI, Some(legacy.into())))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    store
        .auth_apply_validation("shared-github", 1, true, None, NOW)
        .await
        .unwrap();
    let body = get_json(&app, URI).await;
    assert_eq!(body["identity"], "app/Iv23legacy/installation/34");
    assert_eq!(
        body["target_allowlist"],
        serde_json::json!(["https://github.com/Indexyz"])
    );
    assert!(body.get("app_id").is_none());
    assert!(body.get("schema_version").is_none());
    let impact = get_json(&app, &format!("{URI}/impact")).await;
    assert_eq!(impact["liveFleets"], serde_json::json!([]));
    for (scope, expected) in [
        ("auth.read", StatusCode::OK),
        ("auth.write", StatusCode::FORBIDDEN),
    ] {
        let request = axum::http::Request::builder()
            .uri(format!("{URI}/impact"))
            .header("authorization", common::oidc::bearer(scope))
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            expected
        );
    }
    replace(&app, V2.into()).await;
    assert_eq!(get_json(&app, URI).await["desired"]["state"], "Validating");
    store
        .auth_apply_validation("shared-github", 2, false, Some("AuthIdentityMismatch"), NOW)
        .await
        .unwrap();
    let body = get_json(&app, URI).await;
    assert_eq!(body["active"]["revision"], 1);
    assert_eq!(body["active"]["schema_version"], 1);
    assert_eq!(body["identity"], "app/Iv23legacy/installation/34");
    assert_eq!(body["desired"]["revision"], 2);
    assert_eq!(body["desired"]["state"], "Rejected");
    assert_eq!(body["desired"]["reason"], "AuthIdentityMismatch");
    assert_eq!(body["desired"]["bindings"], serde_json::json!([]));
    let revision = get_json(&app, &format!("{URI}/revisions/2")).await;
    assert_eq!(revision["schema_version"], 2);
    assert_eq!(revision["state"], "Rejected");
    assert_eq!(revision["reason"], "AuthIdentityMismatch");
}

#[tokio::test]
async fn real_http_reports_bounded_binding_health_live_impact_and_full_fleet_pins() {
    let (app, store, _) = build_app_with_scan().await;
    assert_eq!(
        app.clone()
            .oneshot(authorized("PUT", URI, Some(V2.into())))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    promote(&store).await;
    let initial = get_json(&app, URI).await;
    let bindings = initial["active"]["bindings"].as_array().unwrap();
    assert_eq!(
        bindings[0]["health"], "Unknown",
        "expired binding cannot imply current access"
    );
    assert_eq!(bindings[1]["health"], "Validated");
    assert_eq!(bindings[1]["valid_until_ms"], NOW + 60_000);
    assert_eq!(bindings[1]["reason"], "CandidateValidationOnly");
    seed_fleet(&store, "repo-blocked").await;
    seed_fleet(&store, "repo-pending").await;

    let row = store
        .fleet_auth_context_get("repo-blocked")
        .await
        .unwrap()
        .unwrap();
    let mut context: ResolvedAuthContext =
        serde_json::from_str(row.desired_context_json.as_deref().unwrap()).unwrap();
    context.repository_id = Some(300);
    context.repository_owner_id = Some(200);
    let expectation = AuthHandoffExpectation {
        mutation_fence: store
            .fleet_get("repo-blocked")
            .await
            .unwrap()
            .unwrap()
            .mutation_fence,
        desired_context_json: row.desired_context_json,
    };
    assert_eq!(
        store
            .handoff_acknowledge(
                "repo-blocked",
                "shared-github",
                1,
                Some(&serde_json::to_string(&context).unwrap()),
                &expectation
            )
            .await
            .unwrap(),
        FleetContextAck::Acknowledged
    );
    store
        .fleet_auth_context_block("repo-blocked", "InstallationSuspended", NOW + 15_000, NOW)
        .await
        .unwrap();
    let impact = get_json(&app, &format!("{URI}/impact")).await;
    let fleets = impact["liveFleets"].as_array().unwrap();
    assert_eq!(fleets.len(), 2);
    assert!(fleets.iter().any(
        |fleet| fleet["fleetKey"] == "repo-blocked" && fleet["target"]["repository"] == "repo"
    ));

    // A failed candidate stays separate from active binding conditions.
    replace(&app, V2.replace("fixture-pem", "rotation-pem")).await;
    store
        .auth_apply_validation("shared-github", 2, false, Some("AuthIdentityMismatch"), NOW)
        .await
        .unwrap();
    let body = get_json(&app, URI).await;
    assert_eq!(body["desired"]["state"], "Rejected");
    assert_eq!(body["active"]["revision"], 1);
    assert_eq!(body["active"]["bindings"][0]["health"], "Unknown");
    assert_eq!(body["active"]["bindings"][1]["health"], "Degraded");
    assert_eq!(
        body["active"]["bindings"][1]["affected_fleets"],
        serde_json::json!(["repo-blocked"])
    );
    let status = get_json(&app, "/api/v1/fleets/repo-blocked/status").await;
    let rollout = &status["githubAuth"]["context"];
    assert_eq!(rollout["state"], "Blocked");
    assert_eq!(rollout["reason"], "InstallationSuspended");
    for route in ["desiredRoute", "observedRoute"] {
        assert_eq!(rollout[route]["profileKey"], "shared-github");
        assert_eq!(rollout[route]["revision"], 1);
        assert_eq!(rollout[route]["githubHost"], "github.com");
        assert_eq!(rollout[route]["appId"], "4863460");
        assert_eq!(rollout[route]["account"]["kind"], "user");
        assert_eq!(rollout[route]["account"]["id"], 200);
        assert_eq!(rollout[route]["installationId"], 22);
        if route == "observedRoute" {
            assert_eq!(rollout[route]["repositoryId"], 300);
            assert_eq!(rollout[route]["repositoryOwnerId"], 200);
        } else {
            assert!(
                rollout[route]["repositoryId"].is_null(),
                "desired intent has not acquired observed pins"
            );
        }
    }
    store
        .auth_route_observation_report(AuthRouteObservation {
            fleet_key: "repo-pending".into(),
            context: context.clone(),
            checked_at_ms: NOW,
            valid_until_ms: NOW + 15_000,
            healthy: false,
            reason: Some("TokenPermissionDenied".into()),
        })
        .await
        .unwrap();
    let observed_health = get_json(&app, URI).await;
    assert_eq!(
        observed_health["active"]["bindings"][1]["reason"],
        "TokenPermissionDenied"
    );
    assert_eq!(
        observed_health["active"]["bindings"][1]["valid_until_ms"],
        NOW + 15_000
    );
    assert_eq!(
        observed_health["active"]["bindings"][0]["health"],
        "Unknown"
    );
    let serialized = body.to_string();
    assert!(!serialized.contains("fixture-pem") && !serialized.contains("rotation-pem"));
}
