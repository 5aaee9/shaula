//! HTTP authorization, exact identity and sanitized failures for discovery.
use crate::common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{TemplateSource, TemplateVariables};
use shaula_http::router::{AppState, ArtifactPublisher};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

#[derive(Default)]
struct Library {
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl ArtifactPublisher for Library {
    async fn publish(&self, _: &[u8], _: &str) -> CoreResult<u64> {
        Err(CoreError::new(
            ReasonCode::StorageUnavailable,
            "PRIVATE_DATABASE_PATH",
        ))
    }
    async fn sources(&self) -> CoreResult<Vec<TemplateSource>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![TemplateSource {
            key: "docker".into(),
            artifact_digest: digest("aa"),
            platform: "docker".into(),
            engine_ref: "terraform".into(),
        }])
    }
    async fn variables(&self, key: &str) -> CoreResult<Option<TemplateVariables>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if key == digest("aa") {
            return Ok(Some(TemplateVariables {
                artifact_digest: key.into(),
                available: false,
                reason: Some("legacy untyped declaration".into()),
                bindings: vec![],
                parameters: vec![],
            }));
        }
        if key == digest("bb") {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "SECRET_FROM_SCHEMA",
            ));
        }
        if key == digest("cc") {
            return Err(CoreError::new(
                ReasonCode::StorageUnavailable,
                "PRIVATE_DATABASE_PATH",
            ));
        }
        Ok(None)
    }
}

fn digest(pair: &str) -> String {
    format!("sha256:{}", pair.repeat(32))
}

async fn app(library: Arc<Library>) -> Router {
    let (_, _, _, service) = common::build_app_with_service().await;
    shaula_http::router::build_router(AppState {
        auth_installation_link: None,
        fleets: service.clone(),
        profiles: service.clone(),
        health: service,
        oidc: common::oidc::oidc().await,
        body_limit: 64 << 20,
        request_body_limit: 1 << 20,
        jobs: None,
        logs: None,
        artifact_publisher: library,
    })
}

#[tokio::test]
async fn discovery_authorizes_before_storage_and_is_private(
) -> Result<(), Box<dyn std::error::Error>> {
    let library = Arc::new(Library::default());
    let app = app(library.clone()).await;
    let path = format!("/api/v1/template-artifacts/{}/variables", digest("aa"));
    for path in ["/api/v1/template-sources", path.as_str()] {
        for (scope, status) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some("fleet.write"), StatusCode::FORBIDDEN),
        ] {
            let mut request = Request::builder().uri(path);
            if let Some(scope) = scope {
                request = request.header("authorization", common::oidc::bearer(scope));
            }
            assert_eq!(
                app.clone()
                    .oneshot(request.body(Body::empty())?)
                    .await?
                    .status(),
                status
            );
        }
    }
    assert_eq!(library.reads.load(Ordering::SeqCst), 0);
    for (path, field, expected) in [
        (
            "/api/v1/template-sources".into(),
            "sources",
            serde_json::json!([{"key":"docker","artifactDigest":digest("aa"),"platform":"docker","engineRef":"terraform"}]),
        ),
        (path, "artifactDigest", serde_json::json!(digest("aa"))),
    ] {
        let response = app
            .clone()
            .oneshot(common::authorized("GET", &path, None))
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        let data: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 4096).await?)?;
        assert_eq!(data[field], expected);
    }
    Ok(())
}

#[tokio::test]
async fn discovery_distinguishes_absence_invalidity_and_storage_failure(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = app(Arc::default()).await;
    for (digest, status, code) in [
        ("invalid".into(), StatusCode::NOT_FOUND, "NotFound"),
        (digest("dd"), StatusCode::NOT_FOUND, "NotFound"),
        (digest("bb"), StatusCode::CONFLICT, "VariablesUnavailable"),
        (digest("cc"), StatusCode::INTERNAL_SERVER_ERROR, "Internal"),
    ] {
        let response = app
            .clone()
            .oneshot(common::authorized(
                "GET",
                &format!("/api/v1/template-artifacts/{digest}/variables"),
                None,
            ))
            .await?;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await?;
        let text = std::str::from_utf8(&bytes)?;
        assert!(!text.contains("SECRET_FROM_SCHEMA") && !text.contains("PRIVATE_DATABASE_PATH"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes)?["code"],
            code
        );
    }
    Ok(())
}

#[tokio::test]
async fn archive_storage_failure_is_sanitized_and_retryable(
) -> Result<(), Box<dyn std::error::Error>> {
    let response = app(Arc::default())
        .await
        .oneshot(common::authorized(
            "PUT",
            &format!("/api/v1/template-artifacts/{}", digest("aa")),
            None,
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), 4096).await?;
    assert!(!std::str::from_utf8(&body)?.contains("PRIVATE_DATABASE_PATH"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body)?["code"],
        "Internal"
    );
    Ok(())
}
