use super::*;
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    operation_log::*,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

struct Backend {
    reads: AtomicUsize,
    pending: bool,
}
#[async_trait]
impl SetupInfoAuthorizer for Backend {
    async fn authorize(&self, generation: &str, token: &str, _: i64) -> CoreResult<bool> {
        Ok(generation == "approved" && token == "a".repeat(64))
    }
}
#[async_trait]
impl OperationLogReadPort for Backend {
    async fn list_invocations(&self, _: &str) -> CoreResult<Vec<Invocation>> {
        Ok(Vec::new())
    }
    async fn read_page(&self, _: &str, _: LogQuery) -> CoreResult<LogPage> {
        Err(CoreError::new(ReasonCode::Internal, "not used"))
    }
    async fn setup_projection(&self, _: &str) -> CoreResult<SetupProjection> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(SetupProjection {
            status: if self.pending { "pending" } else { "ready" }.into(),
            detail: Some("approved apply output".into()),
            invocation_id: None,
            content_version: None,
            partial: false,
        })
    }
}

fn config() -> SetupInfoConfig {
    SetupInfoConfig {
        listen: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        advertised_origin: "https://runner-log.test".into(),
        capability_ttl_seconds: 3600,
        wait_seconds: 60,
        requests_per_second: 2,
        burst: 4,
        max_concurrent_requests: 64,
    }
}

fn request(
    generation: &str,
    token: Option<&str>,
    method: &str,
) -> Result<Request<Body>, axum::http::Error> {
    let mut builder = Request::builder()
        .uri(format!("/runner/v1/generations/{generation}/setup-info"))
        .method(method);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty())
}

#[tokio::test]
async fn authorization_precedes_log_read_and_every_response_has_private_headers() -> TestResult {
    let backend = Arc::new(Backend {
        reads: AtomicUsize::new(0),
        pending: false,
    });
    let router = router(&config(), backend.clone(), backend.clone());
    for (generation, token, method, status) in [
        ("approved", None, "GET", StatusCode::UNAUTHORIZED),
        (
            "other",
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            "GET",
            StatusCode::UNAUTHORIZED,
        ),
        ("approved", None, "POST", StatusCode::NOT_FOUND),
    ] {
        let response = router
            .clone()
            .oneshot(request(generation, token, method)?)
            .await?;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["Cache-Control"], "private, no-store");
        assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
    }
    assert_eq!(backend.reads.load(Ordering::Relaxed), 0);
    let token = "a".repeat(64);
    let response = router
        .oneshot(request("approved", Some(&token), "GET")?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await?.to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value[0]["Group"], "Terraform apply (runner provisioning)");
    assert_eq!(value[0]["Detail"], "approved apply output");
    Ok(())
}

#[tokio::test]
async fn pending_retries_are_rate_limited() -> TestResult {
    let backend = Arc::new(Backend {
        reads: AtomicUsize::new(0),
        pending: true,
    });
    let router = router(&config(), backend.clone(), backend);
    let token = "a".repeat(64);
    for expected in [202, 202, 202, 202, 429] {
        let response = router
            .clone()
            .oneshot(request("approved", Some(&token), "GET")?)
            .await?;
        assert_eq!(response.status().as_u16(), expected);
        assert_eq!(response.headers()["Retry-After"], "1");
    }
    Ok(())
}

#[tokio::test]
async fn ambiguous_authorization_does_not_read_logs() -> TestResult {
    let backend = Arc::new(Backend {
        reads: AtomicUsize::new(0),
        pending: false,
    });
    let router = router(&config(), backend.clone(), backend.clone());
    let mut request = request("approved", Some(&"a".repeat(64)), "GET")?;
    request
        .headers_mut()
        .append("Authorization", "Bearer another-token".parse()?);
    let response = router.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(backend.reads.load(Ordering::Relaxed), 0);
    Ok(())
}
