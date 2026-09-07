#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::attestation_harness::put_template_profile;
use common::*;
#[tokio::test]
async fn fleet_delete_commission_flow_and_preconditions() {
    let (app, control_plane, engine) = build_app_with_scan().await;

    // Seed: auth + template + attestation + fleet create.
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
    assert_eq!(
        put_template_profile(
            &app,
            "k8s-linux",
            TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest),
        )
        .await,
        StatusCode::ACCEPTED
    );
    control_plane
        .periodic_scan(1_800_000_001_000)
        .await
        .unwrap();

    // Activate the auth profile via the validation outcome before fleet
    // admission.
    control_plane
        .auth_apply_validation("prod-app", 1, true, None, 1_800_000_001_500)
        .await
        .unwrap();
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

    // Create the fleet and capture its ETag.
    let request = put_with_idempotency(
        "/api/v1/fleets/linux-x64",
        "create-fleet",
        FLEET_BODY.to_string(),
    );
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let etag = response
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // DELETE without If-Match → 428.
    let response = app
        .clone()
        .oneshot(authorized("DELETE", "/api/v1/fleets/linux-x64", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);

    // DELETE with stale If-Match → 412.
    let request = Request::builder()
        .method("DELETE")
        .uri("/api/v1/fleets/linux-x64")
        .header("authorization", crate::common::oidc::bearer("fleet.retire"))
        .header("if-match", "\"wrong:0\"")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);

    // DELETE with the current ETag and an idempotency key → 202.
    let request = Request::builder()
        .method("DELETE")
        .uri("/api/v1/fleets/linux-x64")
        .header("authorization", crate::common::oidc::bearer("fleet.retire"))
        .header("if-match", format!("\"{etag}\""))
        .header("idempotency-key", "delete-fleet")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "decommission must be accepted: {}",
        String::from_utf8_lossy(&body)
    );

    // Subsequent PUT on a decommissioning fleet → 410 Gone.
    let request = put_with_idempotency(
        "/api/v1/fleets/linux-x64",
        "revive-fleet",
        FLEET_BODY.to_string(),
    );
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::GONE,
        "a decommissioning fleet must not accept new revisions"
    );

    // Exact DELETE retry (same idempotency key) replays the original result.
    let request = Request::builder()
        .method("DELETE")
        .uri("/api/v1/fleets/linux-x64")
        .header("authorization", crate::common::oidc::bearer("fleet.retire"))
        .header("if-match", format!("\"{etag}\""))
        .header("idempotency-key", "delete-fleet")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED, "delete replay");

    // Tombstoned fleet GET → 410.
    let response = app
        .oneshot(authorized("GET", "/api/v1/fleets/linux-x64/status", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("Decommissioning"),
        "phase must read Decommissioning while draining"
    );
}

#[tokio::test]
async fn decommission_commit_waits_for_an_in_flight_admission_claim() {
    let (app, control_plane, engine, service) = build_app_with_service().await;

    // Seed: auth + template + attestation + fleet create (identical to
    // the commission flow test above).
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
    assert_eq!(
        put_template_profile(
            &app,
            "k8s-linux",
            TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest),
        )
        .await,
        StatusCode::ACCEPTED
    );
    control_plane
        .periodic_scan(1_800_000_001_000)
        .await
        .unwrap();
    control_plane
        .auth_apply_validation("prod-app", 1, true, None, 1_800_000_001_500)
        .await
        .unwrap();
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

    let request = put_with_idempotency(
        "/api/v1/fleets/linux-x64",
        "create-fleet",
        FLEET_BODY.to_string(),
    );
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let etag = response
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Hold the fleet's admission claim the way an in-flight apply does
    // (between its durable ApplyStarting record and the apply
    // terminating — R5-02).
    let claim = service.effect_gates().acquire_claim("linux-x64").await;

    let request = Request::builder()
        .method("DELETE")
        .uri("/api/v1/fleets/linux-x64")
        .header("authorization", crate::common::oidc::bearer("fleet.retire"))
        .header("if-match", format!("\"{etag}\""))
        .header("idempotency-key", "delete-waiting")
        .body(Body::empty())
        .unwrap();
    let delete = tokio::spawn(app.clone().oneshot(request));
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        !delete.is_finished(),
        "the decommission commit must wait for the in-flight admission claim"
    );

    // The apply terminates: its claim drops and the decommission commit
    // proceeds to the normal accepted outcome.
    drop(claim);
    let response = delete.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}
