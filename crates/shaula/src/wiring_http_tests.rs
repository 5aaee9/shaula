//! G9 composition test: the FULL production surface — HTTP publication,
//! the REAL scheduling loop running the REAL v2 worker against the
//! scripted GitHub mock, then HTTP correction — proves a rejected
//! wrong-App Candidate never becomes the identity authority.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::wiring_tests::{test_plane, tick_and_drain, wiring_with, TestPlane};
use axum::body::Body;
use axum::http::Request;
use shaula_core::registry::ControlPlaneStore;
use shaula_daemon::service::ControlPlane;
use shaula_http::router::AppState;
use std::sync::Arc;
use tower::ServiceExt;
/// The HTTPS identity-provider fixture used by every control-plane test.
#[path = "../../shaula-http/tests/support/mod.rs"]
mod http_oidc;

use crate::auth_worker_v2::tests::KEY;

const T0: i64 = 1_800_000_000_000;

fn v2_body(app_id: &str) -> String {
    // The App JWT must actually SIGN during validation, so the body
    // carries the real RSA fixture (escaped as \n in JSON).
    let pem = crate::auth_worker_v2::tests::real_pem().replace('\n', "\\n");
    format!(
        r#"{{"kind":"github_app","schema_version":2,"app_id":"{app_id}","private_key":"{pem}","target_policy":[{{"kind":"account_repositories","account_kind":"user","owner":"5aaee9"}}]}}"#
    )
}

fn authorized(method: &str, uri: &str, body: String) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", http_oidc::bearer(http_oidc::SCOPES))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn put_request(uri: &str, body: String, etag: Option<&[u8]>) -> Request<Body> {
    let mut request = authorized("PUT", uri, body);
    match etag {
        Some(etag) => {
            request.headers_mut().insert(
                "if-match",
                axum::http::HeaderValue::from_bytes(etag).unwrap(),
            );
        }
        // A CREATE publication is conditional on the profile not existing.
        None => {
            request
                .headers_mut()
                .insert("if-none-match", axum::http::HeaderValue::from_static("*"));
        }
    }
    request
}

struct NeverPublisher;
#[async_trait::async_trait]
impl shaula_http::router::ArtifactPublisher for NeverPublisher {
    async fn publish(
        &self,
        _bytes: &[u8],
        _declared_digest: &str,
    ) -> shaula_core::error::CoreResult<u64> {
        Err(shaula_core::error::CoreError::new(
            shaula_core::error::ReasonCode::Internal,
            "no artifact uploads in this test",
        ))
    }
}

async fn http_app(control_plane: &TestPlane) -> axum::Router {
    let service = Arc::new(ControlPlane::new(
        control_plane.control_plane.clone(),
        Arc::new(crate::auth_worker_mock::Now),
        b"wiring-http-test".to_vec(),
        100,
        std::path::PathBuf::from("target/unused-engine"),
    ));
    service.set_ready(true);
    shaula_http::router::build_router(AppState {
        fleets: service.clone(),
        profiles: service.clone(),
        health: service,
        oidc: http_oidc::provider().oidc().await,
        body_limit: 64 * 1024 * 1024,
        request_body_limit: 64 * 1024 * 1024,
        jobs: None,
        logs: None,
        artifact_publisher: Arc::new(NeverPublisher),
    })
}

async fn put(
    app: &axum::Router,
    uri: &str,
    body: String,
    etag: Option<&[u8]>,
) -> axum::http::StatusCode {
    let response = app
        .clone()
        .oneshot(put_request(uri, body, etag))
        .await
        .unwrap();
    response.status()
}

/// Captures the current head ETag from the profile READ model.
async fn head_etag(app: &axum::Router, uri: &str) -> Vec<u8> {
    let response = app
        .clone()
        .oneshot(authorized("GET", uri, String::new()))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    response
        .headers()
        .get("etag")
        .map(|v| v.as_bytes().to_vec())
        .expect("profile view carries an ETag")
}

/// A rejected first publication never becomes the profile's App identity.
/// A corrected numeric App declaration can be verified and activated.
#[tokio::test]
async fn http_wrong_app_rejection_does_not_block_corrected_publication() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let plane = test_plane().await;
    let app = http_app(&plane).await;
    let uri = format!("/api/v1/github-auth-profiles/{KEY}");
    let mut wiring = wiring_with(
        &plane.control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;

    assert_eq!(
        put(&app, &uri, v2_body("999999"), None).await,
        axum::http::StatusCode::ACCEPTED,
    );
    tick_and_drain(&mut wiring, T0).await;
    let head = plane
        .control_plane
        .auth_profile_get(KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, None);
    assert_eq!(head.desired_revision, 1);
    let rejected = plane
        .control_plane
        .auth_revision_get(KEY, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rejected.state, "Rejected");
    assert_eq!(rejected.app_id.as_deref(), Some("999999"));
    assert!(plane
        .control_plane
        .auth_bindings_get(KEY, 1)
        .await
        .unwrap()
        .is_empty());

    let etag = head_etag(&app, &uri).await;
    assert_eq!(
        put(&app, &uri, v2_body("4863460"), Some(&etag)).await,
        axum::http::StatusCode::ACCEPTED,
        "a rejected candidate must not prevent correction of the App id",
    );
    tick_and_drain(&mut wiring, T0 + 1_000).await;
    let head = plane
        .control_plane
        .auth_profile_get(KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, Some(2));
}
/// G9 control: a DIFFERENT numeric App against an established ACTIVE
/// identity is still a conflict — the correction loop does not weaken
/// the identity rule.
#[tokio::test]
async fn http_different_app_against_established_identity_still_conflicts() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let plane = test_plane().await;
    let app = http_app(&plane).await;
    let uri = format!("/api/v1/github-auth-profiles/{KEY}");
    let mut wiring = wiring_with(
        &plane.control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;

    assert_eq!(
        put(&app, &uri, v2_body("4863460"), None).await,
        axum::http::StatusCode::ACCEPTED
    );
    tick_and_drain(&mut wiring, T0).await;
    let etag1 = head_etag(&app, &uri).await;

    // App C (yet another numeric id) against the established identity:
    // refused at admission, before any validation runs.
    assert_eq!(
        put(&app, &uri, v2_body("111111"), Some(&etag1)).await,
        axum::http::StatusCode::CONFLICT,
        "a different numeric App is an identity conflict"
    );
    let head = ControlPlaneStore::auth_profile_get(plane.control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.desired_revision, 1, "nothing was published");
}
