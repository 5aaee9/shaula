//! Scripted GitHub + Actions Service router for wire_protocol tests,
//! split to keep files within the 400-line budget (AGENTS.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::routing::{delete, get, post};
use axum::Json;
use axum::Router;
use base64::Engine;
pub(crate) async fn spawn_mock_github() -> String {
    spawn_mock_github_with_label_type("system").await
}

pub(crate) async fn spawn_mock_github_with_label_type(label_type: &'static str) -> String {
    spawn_mock(
        label_type,
        serde_json::json!({
            "totalAvailableJobs": 0, "totalAcquiredJobs": 0, "totalAssignedJobs": 2,
            "totalRunningJobs": 0, "totalRegisteredRunners": 1, "totalBusyRunners": 0,
            "totalIdleRunners": 1
        }),
    )
    .await
}

pub(crate) async fn spawn_mock_github_with_statistics(statistics: serde_json::Value) -> String {
    spawn_mock("system", statistics).await
}

async fn spawn_mock(label_type: &'static str, statistics: serde_json::Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let actions_url = format!("http://{addr}/actions-service");
    let app: Router = Router::new()
        .route(
            "/app/installations/34",
            get(|headers: axum::http::HeaderMap| async move {
                assert_app_jwt(&headers);
                Json(serde_json::json!({
                    "id": 34,
                    "app_id": 123,
                    "account": {"id": 900, "login": "example-org", "type": "Organization"},
                    "repository_selection": "all",
                    "suspended_at": null,
                    "permissions": {
                        "metadata": "read",
                        "administration": "write",
                        "organization_self_hosted_runners": "write"
                    }
                }))
            }),
        )
        .route(
            "/app/installations/34/access_tokens",
            post(|headers: axum::http::HeaderMap| async move {
                assert_app_jwt(&headers);
                (
                    axum::http::StatusCode::CREATED,
                    Json(
                        serde_json::json!({"token": "t-34", "expires_at": "2027-01-01T00:00:00Z"}),
                    ),
                )
            }),
        )
        .route(
            "/orgs/example-org",
            get(|| async { Json(serde_json::json!({"id": 900, "login": "example-org"})) }),
        )
        .route(
            "/orgs/example-org/actions/runners/registration-token",
            post(|headers: axum::http::HeaderMap| async move {
                assert_eq!(headers.get("authorization").unwrap(), "Bearer t-34");
                (
                    axum::http::StatusCode::CREATED,
                    Json(serde_json::json!({"token": "reg-token-1"})),
                )
            }),
        )
        .route(
            "/actions/runner-registration",
            post(move |headers: axum::http::HeaderMap| {
                let actions_url = actions_url.clone();
                async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    assert!(
                        auth.starts_with("RemoteAuth reg-token"),
                        "runner-registration must use RemoteAuth scheme"
                    );
                    Json(serde_json::json!({
                        "url": actions_url,
                        "token": fake_jwt(1_800_000_600),
                    }))
                }
            }),
        )
        .route(
            "/actions-service/_apis/runtime/runnergroups/",
            get(|| async {
                Json(serde_json::json!({
                    "count": 1,
                    "value": [{"id": 7, "name": "Default", "size": 0, "isDefaultGroup": true}]
                }))
            }),
        )
        .route(
            "/actions-service/_apis/runtime/runnerscalesets",
            get(move || async move {
                Json(serde_json::json!({
                    "count": 1,
                    "value": [{
                        "id": 42,
                        "name": "shaula-x64",
                        "runnerGroupId": 7,
                        "runnerGroupName": "Default",
                        "labels": [{"type": label_type, "name": "shaula-x64"}],
                        "RunnerSetting": {},
                        "createdOn": "2026-01-01T00:00:00Z"
                    }]
                }))
            })
            .post(|| async {
                Json(serde_json::json!({
                    "id": 43,
                    "name": "shaula-x64",
                    "runnerGroupId": 7,
                    "runnerGroupName": "Default",
                    "labels": [],
                    "RunnerSetting": {},
                    "createdOn": "2026-01-01T00:00:00Z"
                }))
            }),
        )
        .route(
            "/actions-service/_apis/runtime/runnerscalesets/42/sessions",
            post(move || {
                let statistics = statistics.clone();
                async move {
                    Json(serde_json::json!({
                        "sessionId": "6f9619ff-8b86-d011-b42d-00c04fc964ff",
                        "ownerName": "shaula",
                        "messageQueueUrl": "https://queue.actions.githubusercontent.com/queues/42",
                        "messageQueueAccessToken": "queue-token",
                        "statistics": statistics
                    }))
                }
            }),
        )
        .route(
            "/actions-service/_apis/runtime/runnerscalesets/42/generatejitconfig",
            post(|| async {
                Json(serde_json::json!({
                    "runner": {"id": 9001, "name": "shaula-gen-1", "runnerScaleSetId": 42},
                    "encodedJITConfig": "super-secret-jit"
                }))
            }),
        )
        .route(
            "/actions-service/_apis/distributedtask/pools/0/agents",
            get(|| async {
                Json(serde_json::json!({
                    "count": 1,
                    "value": [{"id": 9001, "name": "shaula-gen-1", "runnerScaleSetId": 42}]
                }))
            }),
        )
        .route(
            "/actions-service/_apis/distributedtask/pools/0/agents/{id}",
            delete(|| async { axum::http::StatusCode::NO_CONTENT }),
        )
        .with_state(());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn assert_app_jwt(headers: &axum::http::HeaderMap) {
    let bearer = headers.get("authorization").unwrap().to_str().unwrap();
    let jwt = bearer.strip_prefix("Bearer ").expect("App JWT bearer");
    let parts: Vec<_> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let decode_part = |part: &str| -> serde_json::Value {
        serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(part)
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(decode_part(parts[0])["alg"], "RS256");
    assert_eq!(decode_part(parts[1])["iss"], "123");
    assert!(!parts[2].is_empty(), "App JWT carries a signature");
}

/// Builds a syntactically valid unsigned JWT with the given `exp`.
fn fake_jwt(exp: i64) -> String {
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256"}"#);
    let payload =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
    format!("{header}.{payload}.not-a-real-signature")
}
