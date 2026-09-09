use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shaula_core::registry::ControlPlaneStore;
use shaula_store::registry_impl::SqliteControlPlane;
use tower::ServiceExt;

use crate::common::{self, attestation_harness::seed_profile};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
pub const PROFILE: &str = "/api/v1/template-profiles/k8s-linux";
pub const UPDATES: &str = "/api/v1/template-profiles/k8s-linux/updates";

pub struct Fixture {
    pub app: axum::Router,
    pub store: Arc<SqliteControlPlane>,
    pub base_digest: String,
    pub target_digest: String,
    pub etag: String,
    pub engine_binary: std::path::PathBuf,
}

impl Fixture {
    pub async fn new() -> TestResult<Self> {
        let (app, store, engine_binary) = common::build_app_with_scan().await;
        let base_digest = seed_profile(&app, &store, "k8s-linux", true).await;
        let fleet = app
            .clone()
            .oneshot(common::authorized(
                "PUT",
                "/api/v1/fleets/build",
                Some(common::FLEET_BODY.to_string()),
            ))
            .await?;
        assert_eq!(fleet.status(), StatusCode::ACCEPTED);
        let (target_digest, bytes) = common::artifact_variants::with_manifest(|manifest| {
            manifest.runtime_policy_digest = "sha256:policy-v2".to_string();
        })?;
        let mut upload = common::authorized(
            "PUT",
            &format!("/api/v1/template-artifacts/{target_digest}"),
            None,
        );
        *upload.body_mut() = Body::from(bytes);
        assert_eq!(
            app.clone().oneshot(upload).await?.status(),
            StatusCode::CREATED
        );
        let response = app
            .clone()
            .oneshot(common::authorized("GET", PROFILE, None))
            .await?;
        let etag = response.headers()["etag"].to_str()?.to_string();
        Ok(Self {
            app,
            store,
            base_digest,
            target_digest,
            etag,
            engine_binary,
        })
    }

    pub fn body(&self) -> serde_json::Value {
        serde_json::json!({"artifact_digest": self.target_digest, "engine_ref": "terraform"})
    }

    pub async fn revision(
        &self,
        revision: i64,
    ) -> TestResult<shaula_core::registry::TemplateRevisionRow> {
        Ok(self
            .store
            .template_revision_get("k8s-linux", revision)
            .await?
            .ok_or("template revision is missing")?)
    }

    pub async fn send(
        &self,
        body: serde_json::Value,
        etag: Option<&str>,
        idem: Option<&str>,
    ) -> TestResult<(StatusCode, String, String)> {
        response(self.app.clone().oneshot(request(body, etag, idem)?).await?).await
    }
}

pub fn request(
    body: serde_json::Value,
    etag: Option<&str>,
    idem: Option<&str>,
) -> TestResult<Request<Body>> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(UPDATES)
        .header("authorization", common::oidc::bearer("template.publish"));
    if let Some(etag) = etag {
        builder = builder.header("if-match", etag);
    }
    if let Some(idem) = idem {
        builder = builder.header("idempotency-key", idem);
    }
    Ok(builder.body(Body::from(body.to_string()))?)
}

pub async fn response(
    response: axum::response::Response,
) -> TestResult<(StatusCode, String, String)> {
    let status = response.status();
    let etag = response
        .headers()
        .get("etag")
        .map(|value| value.to_str())
        .transpose()?
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    Ok((status, String::from_utf8(bytes.to_vec())?, etag))
}
