#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::attestation_harness::put_template_profile;
use common::*;
#[tokio::test]
async fn scan_moves_candidate_to_ready_but_never_active() {
    let (app, control_plane, _engine) = build_app_with_scan().await;

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

    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["status"], "Validating");

    let report = control_plane
        .periodic_scan(1_800_000_001_000)
        .await
        .unwrap();
    assert_eq!(report.candidates_ready, 1);

    // Static validation reaches Ready and stops there: only an exact
    // conformance attestation may activate.
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["status"], "Ready");
    assert!(
        view["activeRevision"].is_null(),
        "static validation must never activate"
    );
}

#[tokio::test]
async fn attestation_activates_then_fleet_admission_binds_exact_pin() {
    let (app, control_plane, engine) = build_app_with_scan().await;

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
    control_plane
        .auth_apply_validation("prod-app", 1, true, None, 1_800_000_001_500)
        .await
        .unwrap();

    // Wrong bindings digest must NOT activate (fail closed).
    let wrong = attest_body(&control_plane, &digest, "bd1_forged", &engine).await;
    let response = app
        .clone()
        .oneshot(attestation_put_request(
            "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/att-bad",
            wrong,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a mismatched attestation is recorded evidence, not a rejection"
    );
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(
        view["status"], "Ready",
        "wrong attestation must not activate"
    );

    // Correct full-subject attestation activates the candidate.
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
    let status = response.status();
    let body_text = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "correct attestation must activate: {}",
        String::from_utf8_lossy(&body_text)
    );
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["status"], "Active");
    assert_eq!(view["activeRevision"], 1);

    // Fleet admission with the bare key resolves to the exact active pin.
    let request = put_with_idempotency(
        "/api/v1/fleets/linux-x64",
        "create-fleet-1",
        FLEET_BODY.to_string(),
    );
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "admission with resolved pin must succeed"
    );
    let etag = response.headers()["etag"].clone();
    assert_eq!(
        response.headers().get("shaula-resource-version"),
        Some(&etag)
    );

    let response = app
        .clone()
        .oneshot(authorized("GET", "/api/v1/fleets/linux-x64", None))
        .await
        .unwrap();
    assert_eq!(
        response.headers().get("shaula-resource-version"),
        Some(&etag)
    );
    assert_eq!(response.headers().get("etag"), Some(&etag));

    // No-op responses must expose the same version for a subsequent edit.
    let mut noop = authorized("PUT", "/api/v1/fleets/linux-x64", Some(FLEET_BODY.into()));
    noop.headers_mut().remove("if-none-match");
    noop.headers_mut().insert("if-match", etag.clone());
    let response = app.clone().oneshot(noop).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("shaula-resource-version"),
        Some(&etag)
    );
    assert_eq!(response.headers().get("etag"), Some(&etag));

    let fleet = get_json(&app, "/api/v1/fleets/linux-x64").await;
    assert_eq!(fleet["resolved"]["authDesired"]["profileKey"], "prod-app");
    assert_eq!(fleet["metadata"]["revision"], 1);
}

#[tokio::test]
async fn forged_subject_members_cannot_activate() {
    let (app, control_plane, engine) = build_app_with_scan().await;

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

    let expected = expected_bindings_digest("k8s-linux", 1);
    // The honest body, then three single-member forgeries: provider
    // version, engine binary digest and suite. A subject the registry
    // cannot reproduce is recorded as MISMATCHED EVIDENCE (R6-08: 201 +
    // audited, subject_verified=false) but must never activate — and
    // each forgery carries its OWN attestation key, because the same
    // key with a different body would be a replay conflict instead.
    let mut forgeries: Vec<(String, String)> = Vec::new();
    forgeries.push(("providers".into(), {
        let honest = attest_body(&control_plane, &digest, &expected, &engine).await;
        let value: serde_json::Value = serde_json::from_str(&honest).unwrap();
        let mut subject = value["subject"].clone();
        subject["providers"][0]["version"] = serde_json::json!("9.9.9");
        let mut value = value;
        value["subject"] = subject;
        value.to_string()
    }));
    forgeries.push(("binary".into(), {
        let honest = attest_body(&control_plane, &digest, &expected, &engine).await;
        let mut value: serde_json::Value = serde_json::from_str(&honest).unwrap();
        value["subject"]["engine"]["binary_digest"] = serde_json::json!(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        );
        value.to_string()
    }));
    forgeries.push(("suite".into(), {
        let honest = attest_body(&control_plane, &digest, &expected, &engine).await;
        let mut value: serde_json::Value = serde_json::from_str(&honest).unwrap();
        value["subject"]["suite"]["version"] = serde_json::json!("v2");
        value["suite"]["version"] = serde_json::json!("v2");
        value.to_string()
    }));

    for (member, body) in forgeries {
        let response = app
            .clone()
            .oneshot(attestation_put_request(
                &format!("/api/v1/template-profiles/k8s-linux/revisions/1/attestations/att-forged-{member}"),
                body,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "forged {member} is durable mismatched evidence (R6-08)"
        );
    }
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(
        view["status"], "Ready",
        "no forged attestation may activate"
    );

    // The honest subject still activates after the forgeries were
    // refused.
    let attest = attest_body(&control_plane, &digest, &expected, &engine).await;
    let response = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/att-good",
            Some(attest),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn incompatible_candidate_cannot_slip_into_a_pending_incarnation() {
    let (app, _control_plane, _engine) = build_app_with_scan().await;

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
    // A DOCKER artifact for the same profile key — never scanned, so
    // its platform/contract were never recorded on the revision row.
    let (docker_digest, docker_bytes) = fixture_artifact_docker();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{docker_digest}"))
        .header(
            "authorization",
            crate::common::oidc::bearer("template.publish template.attest"),
        )
        .body(Body::from(docker_bytes))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );

    // r1: the kubernetes incarnation founder.
    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest);
    assert_eq!(
        put_template_profile(&app, "k8s-linux", body).await,
        StatusCode::ACCEPTED
    );
    // The docker artifact must be refused even though r1's scan has not
    // recorded any platform yet: the FOUNDER's manifest is the authority
    // (F09, spec 0005 §5).
    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &docker_digest);
    let status = put_template_profile(&app, "k8s-linux", body).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an incompatible artifact must not join the kubernetes incarnation"
    );
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["desiredRevision"], 1, "no revision 2 may exist");
}
