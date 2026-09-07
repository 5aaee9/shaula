//! Attestation HTTP harness shared by the attestation convergence
//! suites, split out to keep every file within the 400-line limit
//! (AGENTS.md).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

/// Seeds auth + artifact + profile `key` at revision 1 and drives the
/// scan; returns the artifact digest. `activate_auth` validates the
/// shared auth profile exactly once (the second call would fence-conflict).
pub async fn seed_profile(
    app: &axum::Router,
    control_plane: &std::sync::Arc<shaula_store::registry_impl::SqliteControlPlane>,
    key: &str,
    activate_auth: bool,
) -> String {
    // R10-05: the auth PUT carries the CORRECT precondition — create
    // (If-None-Match: *) when the profile is fresh, If-Match on the
    // current ETag when it already exists (second seed of the same
    // fixture set).
    let probe = app
        .clone()
        .oneshot(authorized(
            "GET",
            "/api/v1/github-auth-profiles/prod-app",
            None,
        ))
        .await
        .unwrap();
    let mut builder = Request::builder()
        .method("PUT")
        .uri("/api/v1/github-auth-profiles/prod-app")
        .header("x-shaula-backend-auth", "test-backend-token-0123456789")
        .header("x-shaula-actor", "ops")
        .header("x-shaula-scopes", "auth.read,auth.write");
    if probe.status() == StatusCode::NOT_FOUND {
        builder = builder.header("if-none-match", "*");
    } else {
        let etag = probe
            .headers()
            .get("etag")
            .cloned()
            .unwrap_or_else(|| axum::http::HeaderValue::from_static("*"));
        builder = builder.header("if-match", etag);
    }
    let request = builder.body(Body::from(AUTH_PUT_BODY)).unwrap();
    let put_status = app.clone().oneshot(request).await.unwrap().status();
    assert!(
        put_status == StatusCode::ACCEPTED || put_status == StatusCode::CONFLICT,
        "auth seed: {put_status} (CONFLICT = same principal already accepted)"
    );
    let (digest, bytes) = fixture_artifact();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{digest}"))
        .header("x-shaula-backend-auth", "test-backend-token-0123456789")
        .header("x-shaula-actor", "publisher")
        .header("x-shaula-scopes", "template.publish,template.attest")
        .body(Body::from(bytes))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );
    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest);
    assert_eq!(
        put_template_profile(app, key, body).await,
        StatusCode::ACCEPTED
    );
    control_plane
        .periodic_scan(1_800_000_001_000)
        .await
        .unwrap();
    if activate_auth {
        control_plane
            .auth_apply_validation("prod-app", 1, true, None, 1_800_000_001_500)
            .await
            .unwrap();
    }
    digest
}

pub async fn put_attestation(
    app: &axum::Router,
    profile: &str,
    revision: i64,
    attestation_key: &str,
    body: String,
) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(attestation_put_request(
            &format!(
                "/api/v1/template-profiles/{profile}/revisions/{revision}/attestations/{attestation_key}"
            ),
            body,
        ))
        .await
        .unwrap();
    let status = response.status();
    let text = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&text).into_owned())
}

pub async fn put_attestation_profile(
    app: &axum::Router,
    profile: &str,
    body: String,
) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/template-profiles/{profile}"))
                .header("x-shaula-backend-auth", "test-backend-token-0123456789")
                .header("x-shaula-actor", "ops")
                .header(
                    "x-shaula-scopes",
                    "fleet.read,fleet.write,fleet.retire,template.read,template.publish,template.attest,template.retire,auth.read,auth.write,auth.retire",
                )
                // The create-style precondition so the request reaches the
                // key-validating boundary instead of stopping at 428.
                .header("if-none-match", "*")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let text = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&text).into_owned())
}

/// Publishes with EXPLICIT preconditions for boundary tests: returns the
/// status of a PUT carrying exactly the requested conditional headers.
pub async fn put_template_profile_raw(
    app: &axum::Router,
    key: &str,
    body: String,
    if_none_match: bool,
    if_match: Option<String>,
    idempotency_key: Option<String>,
) -> StatusCode {
    let mut builder = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-profiles/{key}"))
        .header("x-shaula-backend-auth", "test-backend-token-0123456789")
        .header("x-shaula-actor", "ops")
        .header(
            "x-shaula-scopes",
            "template.read,template.publish,template.attest",
        );
    if let Some(key) = &idempotency_key {
        builder = builder.header("idempotency-key", key);
    }
    if if_none_match {
        builder = builder.header("if-none-match", "*");
    }
    if let Some(etag) = &if_match {
        builder = builder.header("if-match", etag);
    }
    app.clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap()
        .status()
}

/// Publishes/republishes a template profile with the CORRECT conditional
/// headers (R9-02): `If-None-Match: *` for a fresh key, `If-Match` on the
/// current ETag otherwise. Returns the response status.
pub async fn put_template_profile(app: &axum::Router, key: &str, body: String) -> StatusCode {
    let current = app
        .clone()
        .oneshot(authorized(
            "GET",
            &format!("/api/v1/template-profiles/{key}"),
            None,
        ))
        .await
        .unwrap();
    let (create, if_match) = if current.status() == StatusCode::NOT_FOUND {
        (true, None)
    } else {
        let view: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(current.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        (
            false,
            Some((
                view["incarnation"].as_str().unwrap().to_string(),
                view["desiredRevision"].as_i64().unwrap(),
            )),
        )
    };
    let mut builder = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-profiles/{key}"))
        .header("x-shaula-backend-auth", "test-backend-token-0123456789")
        .header("x-shaula-actor", "ops")
        .header(
            "x-shaula-scopes",
            "fleet.read,fleet.write,fleet.retire,template.read,template.publish,template.attest,template.retire,auth.read,auth.write,auth.retire",
        );
    if create {
        builder = builder.header("if-none-match", "*");
    }
    if let Some((incarnation, revision)) = &if_match {
        builder = builder.header("if-match", format!("\"{incarnation}:{revision}\""));
    }
    app.clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap()
        .status()
}
