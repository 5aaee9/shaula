#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Differential-style fixture tests against a scripted local mock of the
//! GitHub API and Actions Service, exercising the full credential chain and
//! every port operation the daemon uses.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
#[path = "wire_protocol/wire_mock_router.rs"]
mod wire_mock_router;

#[path = "wire_protocol/statistics.rs"]
mod statistics;

#[path = "wire_protocol/labels.rs"]
mod labels;

use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;
use shaula_core::ports::GitHubAccessPort;
use shaula_core::ports::LookupOutcome;
use shaula_core::ports::PollOutcome;
use shaula_core::ports::RemovalOutcome;
use shaula_core::secret::SecretString;
use shaula_scaleset::auth::Credential;
use shaula_scaleset::client::ScalesetClient;

#[derive(Debug)]
struct FixedClock(AtomicI64);

impl Clock for FixedClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn fixed_clock() -> Arc<FixedClock> {
    Arc::new(FixedClock(AtomicI64::new(1_800_000_000_000)))
}

fn app_client(github_base: String) -> ScalesetClient {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let target = GitHubTarget::organization("example-org").unwrap();
    ScalesetClient::with_local_servers(
        target,
        Credential::GitHubApp {
            client_id: "123".into(),
            installation_id: 34,
            private_key: SecretString::new(
                std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/app.private.pem"),
                )
                .unwrap(),
            ),
        },
        github_base,
        fixed_clock(),
        http,
    )
}

#[tokio::test]
async fn full_auth_chain_and_port_operations() {
    let github_base = wire_mock_router::spawn_mock_github().await;
    let client = app_client(github_base);

    // 1. Runner group resolution through the admin token chain.
    let identity = shaula_core::github::ScaleSetIdentity {
        target: GitHubTarget::organization("example-org").unwrap(),
        runner_group: "Default".into(),
        scale_set_name: "shaula-x64".into(),
    };
    let group_id = client.resolve_runner_group(&identity).await.unwrap();
    assert_eq!(group_id, 7);

    // 2. Lookup finds exactly one compatible scale set.
    match client.lookup_scale_set(&identity, group_id).await.unwrap() {
        LookupOutcome::ExactlyOne(view) => {
            assert_eq!(view.id, 42);
            assert_eq!(view.labels.len(), 1);
            assert_eq!(view.labels[0].name, "shaula-x64");
            assert_eq!(view.labels[0].label_type, "System");
        }
        other => panic!("expected exactly one scale set, got {other:?}"),
    }

    // 3. Session establishment carries initial statistics (level source).
    let session = client.establish_session(42, "shaula").await.unwrap();
    let session = match session {
        shaula_core::ports::EffectOutcome::Definite(s) => s,
        shaula_core::ports::EffectOutcome::Uncertain { summary } => {
            panic!("unexpected uncertainty: {summary}")
        }
    };
    assert_eq!(session.initial_statistics.total_assigned_jobs, 2);
    assert_eq!(session.session_id, "6f9619ff-8b86-d011-b42d-00c04fc964ff");

    // 4. JIT generation for the stable runner name.
    let jit = client.generate_jit(42, "shaula-gen-1").await.unwrap();
    let jit = jit.definite().expect("jit definite");
    assert_eq!(jit.runner.id, 9001);
    assert_eq!(jit.encoded, "super-secret-jit");
    let debug_rendered = format!("{jit:?}");
    assert!(
        !debug_rendered.contains("super-secret-jit"),
        "JIT must redact in Debug"
    );

    // 5. Inventory lookup by stable name.
    match client.get_runner_by_name(42, "shaula-gen-1").await.unwrap() {
        shaula_core::ports::RunnerLookup::ExactlyOne(runner) => {
            assert_eq!(runner.id, 9001);
            assert_eq!(runner.scale_set_id, 42);
        }
        other => panic!("expected runner, got {other:?}"),
    }

    // 6. Safe removal.
    assert_eq!(
        client.remove_runner(9001).await.unwrap(),
        RemovalOutcome::Removed
    );
}

#[tokio::test]
async fn unknown_label_type_cannot_establish_scale_set_identity() {
    let github_base = wire_mock_router::spawn_mock_github_with_label_type("future-type").await;
    let client = app_client(github_base);
    let identity = shaula_core::github::ScaleSetIdentity {
        target: GitHubTarget::organization("example-org").unwrap(),
        runner_group: "Default".into(),
        scale_set_name: "shaula-x64".into(),
    };
    let result = client.lookup_scale_set(&identity, 7).await;
    assert!(matches!(
        result,
        Err(shaula_core::ports::AccessFailure::Unavailable { .. })
    ));
}

#[tokio::test]
async fn poll_202_maps_to_no_message() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app: Router = Router::new()
        .route(
            "/queue/42",
            get(|| async { axum::http::StatusCode::ACCEPTED }),
        )
        .with_state(());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let github_base = wire_mock_router::spawn_mock_github().await;
    let client = app_client(github_base);
    let session = shaula_core::ports::SessionHandle {
        session_id: "s".into(),
        message_queue_url: format!("http://{addr}/queue/42"),
        message_queue_access_token: "queue-token".into(),
        initial_statistics: Default::default(),
    };
    assert!(matches!(
        client.poll_messages(&session, 0, 20).await.unwrap(),
        PollOutcome::NoMessage
    ));
}

#[tokio::test]
async fn poll_401_maps_to_session_expired_not_create() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app: Router = Router::new()
        .route(
            "/queue/42",
            get(|| async { axum::http::StatusCode::UNAUTHORIZED }),
        )
        .with_state(());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let github_base = wire_mock_router::spawn_mock_github().await;
    let client = app_client(github_base);
    let session = shaula_core::ports::SessionHandle {
        session_id: "s".into(),
        message_queue_url: format!("http://{addr}/queue/42"),
        message_queue_access_token: "stale".into(),
        initial_statistics: Default::default(),
    };
    assert!(matches!(
        client.poll_messages(&session, 0, 20).await.unwrap(),
        PollOutcome::SessionExpired
    ));
}

#[tokio::test]
async fn poll_message_batch_parses_typed_job_messages() {
    let batch = serde_json::json!([
        {"messageType": "JobAvailable", "runnerRequestId": 11, "jobId": "j1"},
        {"messageType": "JobStarted", "runnerRequestId": 11, "jobId": "j1", "runnerId": 9001, "runnerName": "r1"},
        {"messageType": "JobCompleted", "runnerRequestId": 11, "jobId": "j1", "runnerId": 9001, "runnerName": "r1", "result": "succeeded"},
        {"messageType": "UnknownType", "data": 1}
    ]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let batch = batch.to_string();
    let app: Router = Router::new()
        .route(
            "/queue/42",
            get(move || {
                let batch = batch.clone();
                async move {
                    Json(shaula_scaleset::wire::RunnerScaleSetMessageResponse {
                        message_id: 5,
                        message_type: "RunnerScaleSetJobMessages".into(),
                        body: batch,
                        statistics: Some(shaula_scaleset::wire::RunnerScaleSetStatistic {
                            total_assigned_jobs: 1,
                            ..Default::default()
                        }),
                    })
                }
            }),
        )
        .with_state(());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let github_base = wire_mock_router::spawn_mock_github().await;
    let client = app_client(github_base);
    let session = shaula_core::ports::SessionHandle {
        session_id: "s".into(),
        message_queue_url: format!("http://{addr}/queue/42"),
        message_queue_access_token: "queue-token".into(),
        initial_statistics: Default::default(),
    };
    let PollOutcome::Message(message) = client.poll_messages(&session, 0, 20).await.unwrap() else {
        panic!("expected message");
    };
    assert_eq!(message.message_id, 5);
    assert_eq!(message.statistics.total_assigned_jobs, 1);
    assert_eq!(message.job_available.len(), 1);
    assert_eq!(message.job_available[0].runner_request_id, 11);
    assert_eq!(message.job_started.len(), 1);
    assert_eq!(message.job_started[0].runner_name, "r1");
    assert_eq!(message.job_completed.len(), 1);
}

#[test]
fn known_job_decode_failure_fails_the_whole_batch() {
    // T12: a KNOWN job type whose payload does not decode must fail the
    // batch (no ACK, message stays queued) — never silently produce an
    // empty job list that a consumer would acknowledge.
    let body = r#"[
        {"messageType": "JobAvailable",
         "data": {"runnerRequestId": "not-a-number", "jobId": "job-1"}}
    ]"#;
    let response = shaula_scaleset::wire::RunnerScaleSetMessageResponse {
        message_id: 42,
        message_type: "RunnerScaleSetJobMessages".into(),
        body: body.to_string(),
        statistics: Some(Default::default()),
    };
    let result = shaula_scaleset::port::parse_job_messages(response);
    assert!(
        result.is_err(),
        "decode failure of a known job type must fail the batch"
    );

    // Unknown message types stay tolerated for forward compatibility.
    let body = r#"[{"messageType": "SomeFutureEvent", "data": {"x": 1}}]"#;
    let response = shaula_scaleset::wire::RunnerScaleSetMessageResponse {
        message_id: 43,
        message_type: "RunnerScaleSetJobMessages".into(),
        body: body.to_string(),
        statistics: Some(Default::default()),
    };
    let message = shaula_scaleset::port::parse_job_messages(response).unwrap();
    assert_eq!(message.message_id, 43);
    assert!(message.job_available.is_empty());
}
