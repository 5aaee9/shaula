#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Method, Response, StatusCode, Uri},
    routing::any,
    Router,
};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[test]
fn scope_paths_encode_path_segments_without_form_urlencoding_spaces() {
    assert_eq!(
        ForgejoScope::Instance.api_path(),
        "/api/v1/admin/actions/runners"
    );
    assert_eq!(
        ForgejoScope::Organization("a/b c+".into()).api_path(),
        "/api/v1/orgs/a%2Fb%20c%2B/actions/runners"
    );
    assert_eq!(
        ForgejoScope::Repository {
            owner: "o".into(),
            name: "r".into(),
        }
        .api_path(),
        "/api/v1/repos/o/r/actions/runners"
    );
}

#[test]
fn runner_labels_accept_forgejo_strings_and_legacy_detailed_values() {
    let runner: Runner = serde_json::from_value(serde_json::json!({
        "id": 1,
        "uuid": "runner-uuid",
        "name": "runner",
        "status": "idle",
        "labels": ["linux", {"name": "docker", "type": "custom"}],
        "ephemeral": true,
        "version": "13.1.0"
    }))
    .expect("runner response should decode");

    assert!(runner.has_labels(&["linux".into(), "docker".into()]));
    assert!(runner.is_idle());
    assert!(!runner.is_active());
    assert!(runner.is_known());
}

#[test]
fn unknown_runner_status_is_not_online_or_idle() {
    let runner: Runner = serde_json::from_value(serde_json::json!({
        "id": 1,
        "uuid": "runner-uuid",
        "name": "runner",
        "status": "paused"
    }))
    .expect("runner response should decode");

    assert_eq!(runner.status_kind(), RunnerStatus::Unknown("paused".into()));
    assert!(!runner.is_idle());
    assert!(!runner.is_active());
    assert!(!runner.is_known());
}

#[test]
fn jobs_decode_the_fields_needed_for_unverified_observations() {
    let job: Job = serde_json::from_value(serde_json::json!({
        "id": 7,
        "handle": "opaque-handle",
        "attempt": 2,
        "status": "running",
        "runs_on": ["linux", "docker"],
        "task_id": 9,
        "run_id": 11,
        "repo_id": 13,
        "name": "build",
        "new_field": true
    }))
    .expect("job response should decode");

    assert_eq!(job.handle, "opaque-handle");
    assert_eq!(job.attempt, 2);
    assert_eq!(job.task_id, 9);
    assert_eq!(job.run_id, 11);
    assert_eq!(job.repo_id, 13);
    assert_eq!(job.extra["new_field"], true);
}

#[test]
fn client_debug_never_contains_the_bearer_token() {
    let client = ForgejoClient::new(
        "http://localhost:3000",
        "super-secret-token",
        ForgejoScope::Instance,
    )
    .expect("client configuration should be valid");

    assert!(!format!("{client:?}").contains("super-secret-token"));
}

#[test]
fn client_rejects_unsafe_or_incomplete_configuration() {
    for (url, token, scope) in [
        ("file:///tmp/forgejo", "token", ForgejoScope::Instance),
        ("http://", "token", ForgejoScope::Instance),
        (
            "http://localhost:3000?token=leak",
            "token",
            ForgejoScope::Instance,
        ),
        ("http://localhost:3000", "", ForgejoScope::Instance),
        (
            "http://localhost:3000",
            "token",
            ForgejoScope::Organization("".into()),
        ),
    ] {
        assert!(
            ForgejoClient::new(url, token, scope).is_err(),
            "configuration unexpectedly accepted: {url}"
        );
    }
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: Method,
    uri: Uri,
    authorization: Option<String>,
    body: Vec<u8>,
}

async fn mock_handler(
    State(state): State<MockState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    state
        .requests
        .lock()
        .expect("mock state lock")
        .push(RecordedRequest {
            method: method.clone(),
            uri: uri.clone(),
            authorization: headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            body: body.to_vec(),
        });

    let path = uri.path();
    if method == Method::GET && path.ends_with("/api/v1/version") {
        return json_response(StatusCode::OK, serde_json::json!({"version":"16.0.4"}));
    }
    if method == Method::POST && path.ends_with("/api/v1/admin/actions/runners") {
        return json_response(
            StatusCode::CREATED,
            serde_json::json!({"id": 42, "uuid": "uuid-42", "token": "one-time-token"}),
        );
    }
    if method == Method::GET && path.ends_with("/api/v1/orgs/example/actions/runners") {
        let page = uri
            .query()
            .and_then(|query| query.split('&').find_map(|part| part.strip_prefix("page=")))
            .unwrap_or("1");
        return match page {
            "1" => json_response(
                StatusCode::OK,
                serde_json::json!([{
                    "id": 42,
                    "uuid": "uuid-42",
                    "name": "shaula-generation-1",
                    "status": "idle",
                    "labels": ["linux"],
                    "ephemeral": true,
                    "version": "13.1.0"
                }]),
            ),
            _ => json_response(StatusCode::OK, serde_json::json!([])),
        };
    }
    if method == Method::GET && path.ends_with("/api/v1/orgs/example/actions/runners/jobs") {
        if uri.query().is_some_and(|query| query.contains("labels="))
            && !uri
                .query()
                .is_some_and(|query| query.contains("labels=linux"))
        {
            return json_response(StatusCode::OK, serde_json::Value::Null);
        }
        return json_response(
            StatusCode::OK,
            serde_json::json!([{
                "id": 7,
                "handle": "opaque-handle",
                "attempt": 1,
                "status": "waiting",
                "runs_on": ["linux"],
                "task_id": 0,
                "run_id": 11,
                "repo_id": 13,
                "name": "build"
            }]),
        );
    }
    if method == Method::DELETE && path.ends_with("/api/v1/admin/actions/runners/42") {
        return Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("valid empty response");
    }
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("not found"))
        .expect("valid not-found response")
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .expect("valid JSON response")
}

async fn mock_server() -> (String, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .fallback(any(mock_handler))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let address = listener.local_addr().expect("mock server address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock server");
    });
    (format!("http://{address}/forgejo/"), state)
}

#[test]
fn bootstrap_material_redacts_token_and_preserves_targets() {
    let client = ForgejoClient::new(
        "https://forgejo.example.test/",
        "one-time-secret",
        ForgejoScope::Instance,
    )
    .expect("client configuration should be valid");
    let registration = Registration::new(42, "uuid-42".into(), "one-time-secret".into());
    let material = client
        .bootstrap_material(&registration, &["linux:docker://alpine".into()])
        .expect("bootstrap material should validate");
    assert_eq!(material.labels, vec!["linux:docker://alpine"]);
    assert_eq!(material.token(), "one-time-secret");
    assert!(!format!("{material:?}").contains("one-time-secret"));
}

#[test]
fn demand_snapshot_is_replacement_based_and_running_does_not_add_capacity() {
    use shaula_core::ports::forgejo::{ForgejoDemandSnapshot, ForgejoJob};
    let jobs = vec![
        ForgejoJob {
            id: 1,
            handle: String::new(),
            attempt: 1,
            status: "waiting".into(),
            runs_on: vec!["linux".into()],
            task_id: 0,
            run_id: 0,
            repo_id: 0,
            name: "waiting".into(),
        },
        ForgejoJob {
            id: 2,
            handle: String::new(),
            attempt: 1,
            status: "running".into(),
            runs_on: vec!["linux".into()],
            task_id: 0,
            run_id: 0,
            repo_id: 0,
            name: "running".into(),
        },
    ];
    let snapshot = ForgejoDemandSnapshot::from_jobs(jobs, 10);
    assert_eq!(snapshot.waiting_jobs, 1);
    assert_eq!(snapshot.running_jobs, 1);
    assert_eq!(snapshot.target(2, 8), 3);
    let stale = ForgejoDemandSnapshot::stale_from(&snapshot, 20);
    assert_eq!(stale.waiting_jobs, 1);
    assert!(stale.stale);
}

#[tokio::test]
async fn null_jobs_response_is_an_empty_snapshot() {
    let (base_url, _state) = mock_server().await;
    let client = ForgejoClient::new(
        &base_url,
        "test-token",
        ForgejoScope::Organization("example".into()),
    )
    .expect("client configuration should be valid");
    let jobs = client
        .list_jobs(&[])
        .await
        .expect("null jobs response should decode");
    assert!(jobs.is_empty());
}

#[tokio::test]
async fn client_uses_scoped_routes_bearer_auth_and_safe_delete_path() {
    let (base_url, state) = mock_server().await;
    let client = ForgejoClient::new(
        &base_url,
        "test-token",
        ForgejoScope::Organization("example".into()),
    )
    .expect("client configuration should be valid")
    .with_page_size(1)
    .expect("page size should be valid");

    let registration = ForgejoClient::new(&base_url, "test-token", ForgejoScope::Instance)
        .expect("client configuration should be valid")
        .register_runner("runner", None)
        .await
        .expect("registration should succeed");
    assert_eq!(registration.id, 42);

    let runners = client
        .list_runners()
        .await
        .expect("inventory should succeed");
    assert_eq!(runners.len(), 1);
    let jobs = client
        .list_jobs(&["linux".into()])
        .await
        .expect("job listing should succeed");
    assert_eq!(jobs[0].handle, "opaque-handle");

    let removal = ForgejoClient::new(&base_url, "test-token", ForgejoScope::Instance)
        .expect("client configuration should be valid")
        .delete_runner(42)
        .await
        .expect("deletion should succeed");
    assert_eq!(removal, Removal::Removed);

    let requests = state.requests.lock().expect("mock state lock").clone();
    let registration_request = requests
        .iter()
        .find(|request| request.method == Method::POST)
        .expect("registration request should be recorded");
    assert_eq!(
        registration_request.authorization.as_deref(),
        Some("Bearer test-token")
    );
    let registration_body: serde_json::Value =
        serde_json::from_slice(&registration_request.body).expect("registration JSON");
    assert_eq!(registration_body["ephemeral"], true);

    let inventory_request = requests
        .iter()
        .find(|request| {
            request.method == Method::GET
                && request
                    .uri
                    .path()
                    .ends_with("/api/v1/orgs/example/actions/runners")
        })
        .expect("inventory request should be recorded");
    assert!(inventory_request
        .uri
        .query()
        .is_some_and(|query| query.contains("visible=false")));

    let delete_request = requests
        .iter()
        .find(|request| request.method == Method::DELETE)
        .expect("delete request should be recorded");
    assert!(delete_request
        .uri
        .path()
        .ends_with("/api/v1/admin/actions/runners/42"));
}
