#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use axum::{
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use shaula_core::{
    github::GitHubTarget, ports::Clock, registry::AuthRevisionRow, secret::SecretString,
};
use shaula_scaleset::{Credential, ScalesetClient};
use std::sync::Arc;

struct Now;
impl Clock for Now {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

async fn server(installation_app_id: i64) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let service = format!("{base}/actions");
    let token = format!(
        "e30.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1800003600}"#)
    );
    let app = Router::new()
        .route("/user", get(|| async { Json(serde_json::json!({"id": 1, "login": "octocat"})) }))
        .route("/app", get(|| async { Json(serde_json::json!({"id": 12, "client_id": "Iv1.test"})) }))
        .route("/app/installations/34", get(move || async move { Json(serde_json::json!({"id": 34, "app_id": installation_app_id})) }))
        .route("/app/installations/34/access_tokens", post(|| async { (axum::http::StatusCode::CREATED, Json(serde_json::json!({"token": "installation", "expires_at": "2027-01-01T00:00:00Z"}))) }))
        .route("/repos/example/repo", get(|| async { Json(serde_json::json!({"id": 700, "name": "repo", "owner": {"id": 100, "login": "example"}})) }))
        .route("/orgs/example/actions/runners/registration-token", post(|| async { (axum::http::StatusCode::CREATED, Json(serde_json::json!({"token": "registration"}))) }))
        .route("/repos/example/repo/actions/runners/registration-token", post(|| async { (axum::http::StatusCode::CREATED, Json(serde_json::json!({"token": "registration"}))) }))
        .route("/actions/runner-registration", post(move || {
            let token = token.clone(); let service = service.clone();
            async move { Json(serde_json::json!({"url": service, "token": token})) }
        }))
        .route("/actions/_apis/runtime/runnergroups/", get(|| async { Json(serde_json::json!({"count": 1, "value": [{"id": 7, "name": "Default"}]})) }));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, task)
}

#[tokio::test]
async fn auth_validation_checks_identity_and_both_target_kinds() {
    let (base, task) = server(12).await;
    let pem = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app.private.pem"),
    )
    .unwrap();
    for target in [
        GitHubTarget::organization("example").unwrap(),
        GitHubTarget::new_repository("example", "repo").unwrap(),
    ] {
        for app in [false, true] {
            let row = AuthRevisionRow {
                profile_key: "profile".into(),
                revision: 1,
                state: "Validated".into(),
                reason: None,
                kind: if app { "github_app" } else { "pat" }.into(),
                app_id: app.then(|| "Iv1.test".into()),
                installation_id: app.then_some(34),
                pat_principal: (!app).then(|| "octocat".into()),
                allowlist_json: "{}".into(),
                schema_version: 1,
                policy_json: None,
                validation_snapshot_json: None,
            };
            let credential = if app {
                Credential::GitHubApp {
                    client_id: "Iv1.test".into(),
                    installation_id: 34,
                    private_key: SecretString::new(pem.clone()),
                }
            } else {
                Credential::Pat(SecretString::new("test-token"))
            };
            let client = ScalesetClient::with_local_servers(
                target.clone(),
                credential,
                base.clone(),
                Arc::new(Now),
                reqwest::Client::new(),
            );
            let result = client.validate_auth(&row).await;
            assert!(
                result.is_ok(),
                "{}",
                result.err().map(|e| e.summary()).unwrap_or_default()
            );
            let mut wrong = row;
            if app {
                wrong.installation_id = Some(35);
            } else {
                wrong.pat_principal = Some("different-user".into());
            }
            assert!(client.validate_auth(&wrong).await.is_err());
        }
    }
    task.abort();
    let _ = task.await;
}
