//! Scripted-GitHub acceptance for target-aware installation resolution
//! (spec 0011 §4.1/§4.2): per-selector discovery endpoints, App/account
//! identity, suspension and permission classification, and bounded
//! transient handling.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::too_many_arguments)]
use axum::{
    routing::{get, post},
    Json, Router,
};
use shaula_core::auth_policy::{AccountKind, TargetSelector};
use shaula_core::ports::Clock;
use shaula_core::secret::SecretString;
use shaula_scaleset::installation::{
    AppInstallationResolver as Resolver, InstallationLookup, MetadataReachability,
};
use std::sync::Arc;

struct Now;
impl Clock for Now {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

const APP_ID: &str = "4863460";

fn installation_json(
    id: i64,
    app_id: i64,
    login: &str,
    account_id: i64,
    account_type: &str,
    suspended: bool,
    selection: &str,
    administration: &str,
    org_runners: &str,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "id": id,
        "app_id": app_id,
        "account": {"id": account_id, "login": login, "type": account_type},
        "repository_selection": selection,
        "permissions": {
            "metadata": "read",
            "administration": administration,
            "organization_self_hosted_runners": org_runners,
        },
    });
    if suspended {
        body["suspended_at"] = serde_json::json!("2026-01-01T00:00:00Z");
    }
    body
}

async fn server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/app",
            get(|| async { Json(serde_json::json!({"id": 4863460, "slug": "shaula"})) }),
        )
        .route(
            "/orgs/Indexyz/installation",
            get(|| async {
                Json(installation_json(
                    11,
                    4863460,
                    "Indexyz",
                    100,
                    "Organization",
                    false,
                    "all",
                    "write",
                    "write",
                ))
            }),
        )
        .route(
            "/users/5aaee9/installation",
            get(|| async {
                Json(installation_json(
                    22, 4863460, "5aaee9", 200, "User", false, "selected", "write", "read",
                ))
            }),
        )
        .route(
            "/users/ghost/installation",
            get(|| async { axum::http::StatusCode::NOT_FOUND }),
        )
        .route(
            "/users/suspended/installation",
            get(|| async {
                Json(installation_json(
                    33,
                    4863460,
                    "suspended",
                    300,
                    "User",
                    true,
                    "all",
                    "write",
                    "read",
                ))
            }),
        )
        .route(
            "/users/thin/installation",
            get(|| async {
                Json(installation_json(
                    44, 4863460, "thin", 400, "User", false, "all", "read", "read",
                ))
            }),
        )
        .route(
            "/users/foreign/installation",
            get(|| async {
                Json(installation_json(
                    55, 1111111, "foreign", 500, "User", false, "all", "write", "read",
                ))
            }),
        )
        .route(
            "/users/typed/installation",
            get(|| async {
                Json(installation_json(
                    66,
                    4863460,
                    "typed",
                    600,
                    "Organization",
                    false,
                    "all",
                    "write",
                    "read",
                ))
            }),
        )
        .route(
            "/users/flaky/installation",
            get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        )
        .route(
            "/repos/Indexyz/proj/installation",
            get(|| async {
                Json(installation_json(
                    11,
                    4863460,
                    "Indexyz",
                    100,
                    "Organization",
                    false,
                    "all",
                    "write",
                    "write",
                ))
            }),
        )
        .route(
            "/app/installations/22/access_tokens",
            post(|| async {
                (
                    axum::http::StatusCode::CREATED,
                    Json(serde_json::json!({"token": "metadata-token", "expires_at": "2027-01-01T00:00:00Z"})),
                )
            }),
        )
        .route(
            "/app/installations/99/access_tokens",
            post(|| async { axum::http::StatusCode::NOT_FOUND }),
        )
        // The installation-token scoped metadata endpoint (spec 0011 §4.1):
        // only a valid installation token of installation 22 may read it.
        .route(
            "/installation/repositories",
            get(|headers: axum::http::HeaderMap| async move {
                use axum::response::IntoResponse;
                let token = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .trim_start_matches("Bearer ")
                    .to_string();
                if token == "metadata-token" {
                    (
                        axum::http::StatusCode::OK,
                        Json(serde_json::json!({"total_count": 0, "repositories": []})),
                    )
                        .into_response()
                } else {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"message": "bad credentials"})),
                    )
                        .into_response()
                }
            }),
        );
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, task)
}

fn resolver(base: String) -> Resolver {
    Resolver::new(base, reqwest::Client::new(), Arc::new(Now))
}

fn key() -> SecretString {
    SecretString::new(
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app.private.pem"),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn app_identity_proof_accepts_the_declared_app_only() {
    let (base, task) = server().await;
    let verification = resolver(base.clone())
        .verify_app(APP_ID, &key())
        .await
        .unwrap();
    assert_eq!(verification.app_id, 4863460);
    let wrong = resolver(base.clone()).verify_app("1111111", &key()).await;
    assert!(wrong.is_err(), "a different app id must not authenticate");
    // A malformed PEM never counts as a transient failure.
    assert!(resolver(base)
        .verify_app(APP_ID, &SecretString::new("junk"))
        .await
        .is_err());
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn org_user_and_repository_selectors_resolve_installations() {
    let (base, task) = server().await;
    let r = resolver(base);
    let org = r
        .installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::organization("Indexyz").unwrap(),
        )
        .await;
    let InstallationLookup::Proven(proof) = org else {
        panic!("org selector must resolve");
    };
    assert_eq!(proof.installation_id, 11);
    assert_eq!(proof.account_id, 100);
    assert_eq!(proof.account_kind, AccountKind::Organization);
    assert_eq!(proof.login, "Indexyz");
    assert!(proof.has_required_permission);

    let user = r
        .installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::account_repositories(AccountKind::User, "5aaee9").unwrap(),
        )
        .await;
    let InstallationLookup::Proven(proof) = user else {
        panic!("user selector must resolve");
    };
    assert_eq!(proof.installation_id, 22);
    assert_eq!(
        proof.repository_selection,
        shaula_core::auth_context::RepositorySelection::Selected
    );

    let repo = r
        .installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::repository("Indexyz", "proj").unwrap(),
        )
        .await;
    assert!(matches!(repo, InstallationLookup::Proven(_)));
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn terminal_classifications_never_become_retries() {
    let (base, task) = server().await;
    let r = resolver(base);
    assert_eq!(
        r.installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::account_repositories(AccountKind::User, "ghost").unwrap()
        )
        .await,
        InstallationLookup::NotFound
    );
    assert_eq!(
        r.installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::account_repositories(AccountKind::User, "suspended").unwrap()
        )
        .await,
        InstallationLookup::Suspended
    );
    // Missing the required `administration` permission is a denial.
    assert_eq!(
        r.installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::account_repositories(AccountKind::User, "thin").unwrap()
        )
        .await,
        InstallationLookup::PermissionDenied
    );
    // A different App's installation is an identity mismatch, never a
    // fallback route.
    assert_eq!(
        r.installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::account_repositories(AccountKind::User, "foreign").unwrap()
        )
        .await,
        InstallationLookup::IdentityMismatch
    );
    // A declared account kind that disagrees with GitHub's account type.
    assert_eq!(
        r.installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::account_repositories(AccountKind::User, "typed").unwrap()
        )
        .await,
        InstallationLookup::IdentityMismatch
    );
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn server_errors_stay_transient_and_metadata_probe_is_bounded() {
    let (base, task) = server().await;
    let r = resolver(base);
    assert_eq!(
        r.installation_for_selector(
            APP_ID,
            &key(),
            &TargetSelector::account_repositories(AccountKind::User, "flaky").unwrap()
        )
        .await,
        InstallationLookup::Transient {
            retry_after_ms: None
        },
        "5xx stays a bounded retry, never a terminal rejection"
    );
    assert_eq!(
        r.installation_metadata_reachable(APP_ID, &key(), 22).await,
        MetadataReachability::Reachable
    );
    assert_eq!(
        r.installation_metadata_reachable(APP_ID, &key(), 99).await,
        MetadataReachability::NotFound
    );
    task.abort();
    let _ = task.await;
}
