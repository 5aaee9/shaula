//! Collection reads expose existing profiles through the authenticated router.

use super::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{authorized, build_app_with_scan, get_json};
use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::AccountKind;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthPromotion, AuthValidationSnapshot, ControlPlaneStore,
    MutationFacts,
};
use tower::ServiceExt;

const COLLECTION: &str = "/api/v1/github-auth-profiles";
const NOW: i64 = 1_800_000_000_000;
const V2: &str = r#"{"kind":"github_app","schema_version":2,"app_id":"4863460","private_key":"collection-private-key-fixture","target_policy":[{"kind":"account_repositories","account_kind":"user","owner":"5aaee9"}]}"#;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn auth_collection_enumerates_redacted_legacy_and_v2_detail_views() -> TestResult {
    let (app, store, _) = build_app_with_scan().await;
    for (key, body) in [("shared-github", V2), ("legacy-pat", common::AUTH_PUT_BODY)] {
        let response = app
            .clone()
            .oneshot(authorized(
                "PUT",
                &format!("{COLLECTION}/{key}"),
                Some(body.into()),
            ))
            .await?;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    store
        .auth_apply_validation("legacy-pat", 1, true, None, NOW)
        .await?;
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
                bindings: vec![AccountBinding {
                    account_id: 200,
                    account_kind: AccountKind::User,
                    login: "5aaee9".into(),
                    installation_id: 22,
                    repository_selection: RepositorySelection::All,
                    validated_at_ms: NOW,
                }],
                snapshot_json: serde_json::to_string(&snapshot)?,
            }),
        )
        .await?;

    let response = app
        .clone()
        .oneshot(authorized("GET", COLLECTION, None))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    let collection: serde_json::Value = serde_json::from_slice(&bytes)?;
    let profiles = collection["profiles"]
        .as_array()
        .ok_or("profile collection must be an array")?;
    assert_eq!(profiles.len(), 2);
    assert_eq!(
        profiles
            .iter()
            .map(|profile| &profile["key"])
            .collect::<Vec<_>>(),
        vec![
            &serde_json::json!("legacy-pat"),
            &serde_json::json!("shared-github")
        ],
        "collection order is by key, not publication order"
    );
    for key in ["legacy-pat", "shared-github"] {
        let detail = get_json(&app, &format!("{COLLECTION}/{key}")).await;
        assert!(profiles.contains(&detail));
    }
    let text = std::str::from_utf8(&bytes)?;
    assert!(!text.contains("collection-private-key-fixture"));
    assert!(!text.contains("github_pat_test_token_bytes"));
    assert!(!text.contains("private_key"));
    assert!(!text.contains("\"token\""));
    Ok(())
}

#[tokio::test]
async fn auth_collection_propagates_a_profile_read_fault_after_successful_key_enumeration(
) -> TestResult {
    let (app, store, _) = build_app_with_scan().await;
    let corrupt_key = "broken-profile";
    for key in ["healthy-profile", corrupt_key] {
        let response = app
            .clone()
            .oneshot(authorized(
                "PUT",
                &format!("{COLLECTION}/{key}"),
                Some(V2.into()),
            ))
            .await?;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    let head = store
        .auth_profile_get(corrupt_key)
        .await?
        .ok_or("profile head missing")?;
    let mut row = store
        .auth_revision_get(corrupt_key, 1)
        .await?
        .ok_or("revision missing")?;
    row.revision = 2;
    row.policy_json = Some("{malformed stored policy".into());
    // Seed an unreadable persisted revision through the existing Store port.
    // No new production escape hatch or alternate SQLite boundary is needed.
    let commit = store
        .commit_auth_revision(
            MutationFacts {
                resource_kind: "github_auth_profile",
                resource_key: corrupt_key.into(),
                incarnation: head.incarnation,
                revision: 2,
                spec_json: String::new(),
                template: None,
                auth_desired: None,
                inputs_digest: String::new(),
                actor: "fixture".into(),
                now: NOW,
                change: shaula_core::registry::ChangeView {
                    id: "corrupt-revision-fixture".into(),
                    resource_kind: "github_auth_profile".into(),
                    resource_key: corrupt_key.into(),
                    revision: 2,
                    kind: "Publish".into(),
                    state: "Pending".into(),
                    reason: None,
                },
                outbox_topic: "profile.auth_validate".into(),
                outbox_payload: "{}".into(),
                idempotency: None,
            },
            row,
            b"corrupt-profile-secret-fixture",
        )
        .await?;
    assert!(commit.is_ok());
    assert_eq!(store.auth_profile_keys().await?.len(), 2);
    for (path, expected) in [
        (format!("{COLLECTION}/healthy-profile"), StatusCode::OK),
        (
            format!("{COLLECTION}/{corrupt_key}"),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (COLLECTION.into(), StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let response = app.clone().oneshot(authorized("GET", &path, None)).await?;
        assert_eq!(response.status(), expected, "{path}");
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
        let text = std::str::from_utf8(&bytes)?;
        assert!(!text.contains("corrupt-profile-secret-fixture"));
        if expected == StatusCode::INTERNAL_SERVER_ERROR {
            let problem: serde_json::Value = serde_json::from_slice(&bytes)?;
            assert!(
                problem.get("profiles").is_none(),
                "faults must not return partial collections"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn auth_collection_requires_auth_read_and_returns_an_explicit_empty_list() -> TestResult {
    let (app, _, _) = build_app_with_scan().await;
    for (scope, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("auth.write"), StatusCode::FORBIDDEN),
        (Some("auth.read"), StatusCode::OK),
    ] {
        let mut request = Request::builder().uri(COLLECTION);
        if let Some(scope) = scope {
            request = request.header("authorization", common::oidc::bearer(scope));
        }
        let response = app.clone().oneshot(request.body(Body::empty())?).await?;
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
            let body: serde_json::Value = serde_json::from_slice(&bytes)?;
            assert_eq!(body, serde_json::json!({"profiles": []}));
        }
    }
    Ok(())
}
