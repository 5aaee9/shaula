#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! End-to-end HTTP control-plane tests through the axum router with the
//! real ControlPlane service and in-memory infrastructure.

#[path = "../../shaula-http/tests/support/mod.rs"]
mod oidc;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use shaula_core::ports::Clock;
use shaula_daemon::service::ControlPlane;
use shaula_store::registry_impl::SqliteControlPlane;
use shaula_store::Store;

#[derive(Debug)]
struct FixedClock(std::sync::atomic::AtomicI64);

impl Clock for FixedClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn fixed_clock() -> Arc<FixedClock> {
    Arc::new(FixedClock(std::sync::atomic::AtomicI64::new(
        1_800_000_000_000,
    )))
}

/// Builds a valid Template artifact tar.gz and publishes it through the
/// store; returns (digest, bytes).
fn fixture_artifact() -> (String, Vec<u8>) {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, content) in [
        ("profile.yaml", "api_version: shaula.io/template-profile/v1\nkind: RunnerTemplateProfile\nplatform: kubernetes\nruntime:\n  protocol: terraform-cli/v1\n  engine: terraform\n  root_module: .\n  required_version: \">= 1.9, < 2.0\"\nbindings_contract: shaula.bindings.kubernetes/v1\nschemas:\n  bindings: schemas/bindings.schema.json\n  parameters: schemas/parameters.schema.json\nmanaged_resource_shape:\n  - role: bootstrap\n    terraform_type: kubernetes_secret_v1\n    exact_count: 1\n  - role: runner\n    terraform_type: kubernetes_pod_v1\n    exact_count: 1\n"),
        (".terraform.lock.hcl", "# lock\n"),
        ("schemas/bindings.schema.json", "{}"),
        ("schemas/parameters.schema.json", "{}"),
        ("main.tf", "resource \"kubernetes_secret_v1\" \"bootstrap\" {}\n"),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, name, content.as_bytes()).unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
    let bytes = encoder.finish().unwrap();
    let digest = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };
    (digest, bytes)
}

async fn build_app() -> axum::Router {
    let tmp = tempfile::tempdir().unwrap();
    let artifact_root = tmp.path().join("artifacts");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&artifact_root).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    // Keep the tempdir alive for the process.
    let tmp_path = tmp.path().to_path_buf();
    std::mem::forget(tmp);

    let store = Store::open(&data_dir.join("test.db")).await.unwrap();
    store.migrate().await.unwrap();
    let control_plane = Arc::new(SqliteControlPlane::new(store, artifact_root.clone()));

    let service = Arc::new(ControlPlane::new(
        control_plane,
        fixed_clock(),
        b"test-bindings-key".to_vec(),
        100,
        {
            // Stand-in engine binary: the attestation binary-digest
            // authority must point at a real, hashable file.
            let engine_dir = tmp_path.join("engine");
            std::fs::create_dir_all(&engine_dir).unwrap();
            let engine = engine_dir.join("terraform.cmd");
            std::fs::write(&engine, "@echo off\r\nexit /b 0\r\n").unwrap();
            engine
        },
    ));
    service.set_ready(true);

    shaula_http::router::build_router(shaula_http::router::AppState {
        fleets: service.clone(),
        profiles: service.clone(),
        health: service,
        oidc: oidc::oidc().await,
        body_limit: 64 * 1024 * 1024,
        request_body_limit: 64 * 1024 * 1024,
        artifact_publisher: Arc::new(TestPublisher {
            root: artifact_root,
        }),
    })
}

struct TestPublisher {
    root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl shaula_http::router::ArtifactPublisher for TestPublisher {
    async fn publish(
        &self,
        bytes: &[u8],
        declared_digest: &str,
    ) -> shaula_core::error::CoreResult<u64> {
        shaula_template::artifact::ArtifactStore::new(self.root.clone())
            .publish(bytes, declared_digest)
            .map(|p| p.size_bytes)
    }
}

fn authorized(method: &str, uri: &str, body: Option<String>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", oidc::bearer("fleet.read fleet.write fleet.retire template.read template.publish template.attest template.retire auth.read auth.write auth.retire"));
    // R10-05: conditional writes are mandatory — test PUTs are all
    // create-style re-assertions, so they carry If-None-Match: *.
    if method == "PUT" {
        builder = builder.header("if-none-match", "*");
    }
    match body {
        Some(body) => builder.body(Body::from(body)).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

const AUTH_PUT_BODY: &str = r#"{
    "kind": "pat",
    "token": "github_pat_test_token_bytes",
    "pat_principal": "octocat",
    "target_allowlist": [{"kind":"organization","owner":"example-org"}]
}"#;

#[tokio::test]
async fn auth_profile_lifecycle_and_secret_redaction() {
    let app = build_app().await;

    // PUT auth profile → 202 with change.
    let response = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/github-auth-profiles/prod-app",
            Some(AUTH_PUT_BODY.into()),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "auth profile PUT must be accepted"
    );
    // R10-05: the durable ETag is the precondition for every later PUT.
    let etag = response
        .headers()
        .get("etag")
        .expect("accepted mutation must carry an ETag")
        .clone();

    // GET returns redacted metadata only.
    let response = app
        .clone()
        .oneshot(authorized(
            "GET",
            "/api/v1/github-auth-profiles/prod-app",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.contains("credential_present")
            || text.contains("credentialPresent")
            || text.contains("true")
    );
    assert!(
        !text.contains("github_pat_test_token_bytes"),
        "secret bytes must never appear"
    );

    // Second PUT with a CURRENT If-Match and a different principal is an
    // identity conflict (409) — the precondition itself now PASSES, so
    // this exercises the real conflict, not the stale-ETag gate.
    let conflict = r#"{
        "kind": "pat",
        "token": "github_pat_other",
        "pat_principal": "someone-else",
        "target_allowlist": [{"kind":"organization","owner":"example-org"}]
    }"#;
    let mut request = authorized(
        "PUT",
        "/api/v1/github-auth-profiles/prod-app",
        Some(conflict.into()),
    );
    // Replace the create-style precondition with the CURRENT If-Match:
    // the precondition then passes and the request reaches the real
    // principal-migration conflict (409).
    request.headers_mut().remove("if-none-match");
    request.headers_mut().insert("if-match", etag);
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "principal migration must require a new profile key"
    );
}

#[tokio::test]
async fn missing_actor_context_rejected_everywhere() {
    let app = build_app().await;
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/fleets/any")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "loopback never synthesizes an actor"
    );

    // Forged identity header is rejected even with a valid backend token.
    let app2 = build_app().await;
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/fleets/any")
        .header("x-shaula-backend-auth", "test-backend-token-0123456789")
        .header("x-shaula-actor", "ops")
        .header("x-forwarded-user", "attacker")
        .body(Body::empty())
        .unwrap();
    let response = app2.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn fleet_put_requires_valid_template_and_auth_references() {
    let app = build_app().await;

    // Reference a nonexistent template profile → 422.
    let fleet_body = r#"{
        "github": {
            "target": {"kind":"organization","owner":"example-org"},
            "auth_profile_ref": "prod-app",
            "scale_set_name": "shaula-x64",
            "runner_group": "Default",
            "labels": ["shaula-x64"]
        },
        "capacity": {"min_runners": 0, "max_runners": 5},
        "template_profile_ref": "missing-template",
        "template_inputs": {}
    }"#;
    let request = Request::builder()
        .method("PUT")
        .uri("/api/v1/fleets/linux-x64")
        .header("authorization", oidc::bearer("fleet.write"))
        .header("if-none-match", "*")
        .body(Body::from(fleet_body.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn livez_and_readyz() {
    let app = build_app().await;
    let response = app
        .clone()
        .oneshot(authorized("GET", "/livez", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .oneshot(authorized("GET", "/readyz", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn artifact_upload_digest_idempotent_and_shape_validated() {
    let app = build_app().await;
    let (digest, bytes) = fixture_artifact();

    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{digest}"))
        .header("authorization", oidc::bearer("template.publish"))
        .body(Body::from(bytes.clone()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "first publish must create"
    );

    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{digest}"))
        .header("authorization", oidc::bearer("template.publish"))
        .body(Body::from(bytes))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "digest-idempotent republish"
    );
}
