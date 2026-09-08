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
use shaula_store::registry_impl::SqliteControlPlane;
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

/// G9: with an ACTIVE LEGACY profile whose stored identity is a
/// client-ID string (any numeric v2 App id is admitted — continuity is
/// proven by the validator), a wrong-App Candidate (numeric B) is
/// REJECTED by the real worker, and the corrected same-App publication
/// (numeric A) is admitted with the latest ETag and promoted — the
/// original active credential stays continuously preserved.
#[tokio::test]
async fn http_wrong_app_rejection_does_not_block_corrected_same_app_publication() {
    let mock = crate::auth_worker_mock::mock_server(false).await;
    let plane = test_plane().await;
    let app = http_app(&plane).await;
    let uri = format!("/api/v1/github-auth-profiles/{KEY}");
    let mut wiring = wiring_with(
        &plane.control_plane,
        crate::auth_worker_mock::endpoints(&mock.base),
    )
    .await;

    // 0. Seed the established ACTIVE identity through the store the way
    // a pre-existing legacy profile looks: a client-ID STRING, not a
    // numeric App id. The mock's /app identity is (id 4863460,
    // client_id "Iv23tester").
    seed_active_legacy_profile(&plane.control_plane).await;
    let original_bytes =
        ControlPlaneStore::auth_credential_bytes(plane.control_plane.as_ref(), KEY, 1)
            .await
            .unwrap()
            .unwrap();

    // 1. Publish the WRONG numeric App B: admitted (a legacy client-ID
    // identity cannot be compared literally), then rejected by the real
    // validator (/app reports 4863460 ≠ 999999).
    let etag1 = head_etag(&app, &uri).await;
    assert_eq!(
        put(&app, &uri, v2_body("999999"), Some(&etag1)).await,
        axum::http::StatusCode::ACCEPTED,
        "wrong-App publication admitted against the legacy identity"
    );
    tick_and_drain(&mut wiring, T0).await;
    let head = ControlPlaneStore::auth_profile_get(plane.control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head.active_revision,
        Some(1),
        "the active credential is untouched"
    );
    assert_eq!(
        head.desired_revision, 2,
        "the rejected candidate is the desired head"
    );
    let rejected = ControlPlaneStore::auth_revision_get(plane.control_plane.as_ref(), KEY, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rejected.app_id.as_deref(),
        Some("999999"),
        "the wrong-App candidate stored"
    );
    assert!(
        ControlPlaneStore::auth_bindings_get(plane.control_plane.as_ref(), KEY, 2)
            .await
            .unwrap()
            .is_empty(),
        "the wrong-App candidate was rejected: no bindings ever froze"
    );
    assert_eq!(
        ControlPlaneStore::auth_credential_bytes(plane.control_plane.as_ref(), KEY, 1)
            .await
            .unwrap()
            .unwrap(),
        original_bytes,
        "the original active credential bytes are continuously preserved"
    );

    // 2. Corrected SAME-App publication (numeric A, proven to be the
    // same App as the stored client-ID) with the latest ETag: admitted
    // (a rejected candidate is history, not authorization) and promoted
    // by the real worker.
    let etag2 = head_etag(&app, &uri).await;
    assert_eq!(
        put(&app, &uri, v2_body("4863460"), Some(&etag2)).await,
        axum::http::StatusCode::ACCEPTED,
        "the corrected same-App publication must be admitted after rejection"
    );
    tick_and_drain(&mut wiring, T0 + 1_000).await;
    let head = ControlPlaneStore::auth_profile_get(plane.control_plane.as_ref(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head.active_revision,
        Some(3),
        "the corrected candidate (rev 3, after the rejected rev 2) validated and activated"
    );
}

/// Seeds an ACTIVE legacy revision (schema 1) whose identity is the
/// client-ID string the mock's `/app` reports — the exact established
/// state the v2 upgrade path upgrades FROM.
async fn seed_active_legacy_profile(control_plane: &Arc<SqliteControlPlane>) {
    use shaula_core::registry::ControlPlaneStore as _;
    use shaula_core::registry::{AuthRevisionRow, MutationFacts};
    let change = shaula_core::registry::ChangeView {
        id: "legacy-create".into(),
        resource_kind: "github_auth_profile".into(),
        resource_key: KEY.into(),
        revision: 1,
        kind: "Put".into(),
        state: "Pending".into(),
        reason: None,
    };
    let facts = MutationFacts {
        resource_kind: "github_auth_profile",
        resource_key: KEY.into(),
        incarnation: "inc-legacy".into(),
        revision: 1,
        spec_json: String::new(),
        template: None,
        auth_desired: None,
        inputs_digest: String::new(),
        actor: "tester".into(),
        now: 1,
        change,
        outbox_topic: "profile.auth_validate".into(),
        outbox_payload: format!(r#"{{"key":"{KEY}","revision":1}}"#),
        idempotency: None,
    };
    let credential = AuthRevisionRow {
        profile_key: KEY.into(),
        revision: 1,
        kind: "github_app".into(),
        app_id: Some("Iv23tester".into()),
        installation_id: Some(11),
        pat_principal: None,
        allowlist_json: String::new(),
        schema_version: 1,
        policy_json: None,
        validation_snapshot_json: None,
        state: "Validating".into(),
        reason: None,
    };
    control_plane
        .commit_auth_revision(
            facts,
            credential,
            crate::auth_worker_v2::tests::real_pem().as_bytes(),
        )
        .await
        .unwrap()
        .unwrap();
    let outcome = ControlPlaneStore::auth_apply_validation_v2(
        control_plane.as_ref(),
        KEY,
        1,
        true,
        None,
        2,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        shaula_core::registry::AuthPromotionOutcome::Promoted,
        "fixture: the legacy identity is active"
    );
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
