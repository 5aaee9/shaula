#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::attestation_harness::put_template_profile;
use common::*;
#[tokio::test]
async fn idempotency_replay_and_conflict() {
    let (app, control_plane, engine) = build_app_with_scan().await;

    // Seed the dependencies.
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/github-auth-profiles/prod-app",
                Some(AUTH_PUT_BODY.into())
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let (digest, bytes) = fixture_artifact();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{digest}"))
        .header(
            "authorization",
            crate::common::oidc::bearer("template.publish template.attest"),
        )
        .body(Body::from(bytes))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );
    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest);
    assert_eq!(
        put_template_profile(&app, "k8s-linux", body).await,
        StatusCode::ACCEPTED
    );
    control_plane
        .periodic_scan(1_800_000_001_000)
        .await
        .unwrap();

    // Apply the (phase-3) credential validation outcome so the auth
    // profile reaches Active before fleet admission.
    common::auth_fixture::promote(control_plane.as_ref(), "prod-app", 1, 1_800_000_001_500)
        .await
        .unwrap();

    // Record independent evidence after the scan activated the revision.
    let expected = expected_bindings_digest("k8s-linux", 1);
    let attest = attest_body(&control_plane, &digest, &expected, &engine).await;
    let response = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/att-1",
            Some(attest),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // First PUT creates the fleet.
    let request = put_with_idempotency("/api/v1/fleets/linux-x64", "key-1", FLEET_BODY.to_string());
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let etag_first = response
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Same key + same body: replay returns the original ETag and revision.
    let request = put_with_idempotency("/api/v1/fleets/linux-x64", "key-1", FLEET_BODY.to_string());
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let etag_replay = response
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(
        etag_first, etag_replay,
        "replay must return the original result"
    );

    // The replay must not have advanced the revision.
    let fleet = get_json(&app, "/api/v1/fleets/linux-x64").await;
    assert_eq!(
        fleet["metadata"]["revision"], 1,
        "replay must not create a revision"
    );

    // Same key + different body: 409 IdempotencyConflict.
    let changed = FLEET_BODY.replace("\"max_runners\": 5", "\"max_runners\": 6");
    let request = put_with_idempotency("/api/v1/fleets/linux-x64", "key-1", changed);
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
