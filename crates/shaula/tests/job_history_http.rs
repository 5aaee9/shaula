//! Management authorization and bounded diagnostics read contracts.
#[path = "../../shaula-http/tests/support/mod.rs"]
mod oidc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use shaula_core::{jobs::*, operation_log::*, CoreError, CoreResult, ReasonCode};
use shaula_daemon::service::ControlPlane;
use shaula_store::{registry_impl::SqliteControlPlane, Store};
use std::sync::Arc;
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct History;

#[async_trait::async_trait]
impl JobsReadPort for History {
    async fn list_jobs(&self, _: JobsQuery) -> Result<JobsPage, JobsReadError> {
        Ok(JobsPage {
            items: vec![],
            next_cursor: None,
        })
    }
    async fn get_job(&self, _: &str) -> Result<Option<JobDetail>, JobsReadError> {
        Ok(None)
    }
    async fn list_generations(
        &self,
        _: GenerationsQuery,
    ) -> Result<GenerationsPage, JobsReadError> {
        Ok(GenerationsPage {
            items: vec![],
            next_cursor: None,
        })
    }
    async fn get_generation(&self, _: &str) -> Result<Option<GenerationDetail>, JobsReadError> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl OperationLogReadPort for History {
    async fn list_invocations(&self, _: &str) -> CoreResult<Vec<Invocation>> {
        Ok(vec![])
    }
    async fn read_page(&self, id: &str, query: LogQuery) -> CoreResult<LogPage> {
        match id {
            "missing" => {
                return Err(CoreError::new(
                    ReasonCode::TargetHiddenOrNotFound,
                    "internal hidden detail",
                ))
            }
            "offline" => {
                return Err(CoreError::new(
                    ReasonCode::StorageUnavailable,
                    "internal hidden detail",
                ))
            }
            _ => {}
        }
        if query.cursor.is_some() {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "internal hidden detail",
            ));
        }
        Ok(LogPage {
            invocation_id: id.into(),
            content_version: "v1".into(),
            capture_status: "partial".into(),
            entries: vec![LogEntry {
                command_ordinal: 1,
                phase: "apply".into(),
                stream: "stderr".into(),
                sequence: 1,
                text: "provider failed: [REDACTED]\n<script>no execution</script>".into(),
                observed_at: 123,
            }],
            next_cursor: None,
            has_gap: true,
            lost_bytes: 12,
        })
    }
    async fn setup_projection(&self, _: &str) -> CoreResult<SetupProjection> {
        Err(CoreError::new(
            ReasonCode::PermissionDenied,
            "management cannot publish setup info",
        ))
    }
}

struct FixedClock;

impl shaula_core::ports::Clock for FixedClock {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

struct NoArtifacts;

#[async_trait::async_trait]
impl shaula_http::router::ArtifactPublisher for NoArtifacts {
    async fn publish(&self, _: &[u8], _: &str) -> CoreResult<u64> {
        Err(CoreError::new(
            ReasonCode::Internal,
            "history reads must not publish artifacts",
        ))
    }
}

struct Fixture {
    // The router owns the ControlPlane and Store; keep their directory alive too.
    router: axum::Router,
    _directory: tempfile::TempDir,
}

async fn app() -> Result<Fixture, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let artifact_root = directory.path().join("artifacts");
    std::fs::create_dir(&artifact_root)?;
    let store = Store::open(&directory.path().join("history.db")).await?;
    store.migrate().await?;
    let control_plane = Arc::new(SqliteControlPlane::new(store, artifact_root));
    let service = Arc::new(ControlPlane::new(
        control_plane,
        Arc::new(FixedClock),
        b"test-bindings-key".to_vec(),
        100,
        directory.path().join("unused-terraform"),
    ));
    service.set_ready(true);
    let router = shaula_http::router::build_router(shaula_http::router::AppState {
        fleets: service.clone(),
        profiles: service.clone(),
        health: service,
        oidc: oidc::oidc().await,
        body_limit: 1024,
        request_body_limit: 1024,
        artifact_publisher: Arc::new(NoArtifacts),
        jobs: Some(Arc::new(History)),
        logs: Some(Arc::new(History)),
    });
    Ok(Fixture {
        router,
        _directory: directory,
    })
}

fn request(path: &str, scopes: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .uri(path)
        .header("authorization", oidc::bearer(scopes))
        .body(Body::empty())
}

#[tokio::test]
async fn log_body_requires_both_read_scopes_without_write_implication() -> TestResult {
    let fixture = app().await?;
    let app = fixture.router.clone();
    for scopes in [
        "fleet.read",
        "logs.read",
        "fleet.write",
        "fleet.write logs.read",
    ] {
        let response = app
            .clone()
            .oneshot(request("/api/v1/invocations/record/logs", scopes)?)
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{scopes}");
    }
    let response = app
        .oneshot(request(
            "/api/v1/invocations/record/logs",
            "fleet.read logs.read",
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let body = axum::body::to_bytes(response.into_body(), 1024).await?;
    let page: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(page["capture_status"], "partial");
    assert!(page["entries"][0]["text"]
        .as_str()
        .ok_or("log entry text missing")?
        .contains("[REDACTED]"));
    Ok(())
}

#[tokio::test]
async fn history_distinguishes_missing_invalid_and_unavailable_without_internal_details(
) -> TestResult {
    let fixture = app().await?;
    let app = fixture.router.clone();
    for (path, status) in [
        ("/api/v1/jobs/missing", StatusCode::NOT_FOUND),
        (
            "/api/v1/generations/missing/invocations",
            StatusCode::NOT_FOUND,
        ),
        ("/api/v1/invocations/missing/logs", StatusCode::NOT_FOUND),
        (
            "/api/v1/invocations/offline/logs",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            "/api/v1/invocations/record/logs?limit_bytes=1048577",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/v1/invocations/record/logs?stream=state",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/v1/invocations/record/logs?cursor=wrong-version",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(path, "fleet.read logs.read")?)
            .await?;
        assert_eq!(response.status(), status, "{path}");
        let body = axum::body::to_bytes(response.into_body(), 2048).await?;
        assert!(!String::from_utf8_lossy(&body).contains("internal hidden detail"));
    }
    let response = app.oneshot(request("/api/v1/jobs", "fleet.read")?).await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn malformed_log_queries_still_require_permission_and_jobs_deep_links_redirect_to_login(
) -> TestResult {
    let fixture = app().await?;
    let app = fixture.router.clone();
    for scopes in ["fleet.read", "fleet.write logs.read"] {
        let response = app
            .clone()
            .oneshot(request(
                "/api/v1/invocations/record/logs?limit_bytes=bad",
                scopes,
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = app
        .clone()
        .oneshot(request(
            "/api/v1/invocations/record/logs?limit_bytes=bad",
            "fleet.read logs.read",
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers()["content-type"]
        .to_str()?
        .contains("application/json"));
    let body = axum::body::to_bytes(response.into_body(), 2048).await?;
    let problem: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(problem["code"], "HistoryQueryInvalid");
    assert_eq!(problem["detail"], "invalid query parameters");
    for path in [
        "/jobs",
        "/jobs/job-1",
        "/jobs/runners/gen-1",
        "/jobs?fleet_key=linux&repository=acme%2Frepo",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response.headers()["location"].to_str()?;
        let query = location.split_once('?').ok_or("missing login query")?.1;
        let returned = reqwest::Url::parse(&format!("https://shaula.test/?{query}"))?
            .query_pairs()
            .find(|(key, _)| key == "return_to")
            .ok_or("missing return target")?
            .1
            .into_owned();
        assert_eq!(returned, path);
    }
    Ok(())
}
