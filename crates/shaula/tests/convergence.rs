#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

use common::attestation_harness::put_template_profile;
use common::*;
#[tokio::test]
async fn scan_automatically_activates_a_statically_valid_candidate() {
    let (app, control_plane, _engine) = build_app_with_scan().await;

    let (digest, bytes) = fixture_artifact();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{digest}"))
        .header(
            "authorization",
            crate::common::oidc::bearer("template.publish"),
        )
        .body(Body::from(bytes))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );

    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest);
    let mut publication = authorized("PUT", "/api/v1/template-profiles/k8s-linux", Some(body));
    publication.headers_mut().insert(
        "authorization",
        common::oidc::bearer("template.publish").parse().unwrap(),
    );
    assert_eq!(
        app.clone().oneshot(publication).await.unwrap().status(),
        StatusCode::ACCEPTED
    );

    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["status"], "Validating");

    let report = control_plane
        .periodic_scan(1_800_000_001_000)
        .await
        .unwrap();
    assert_eq!(report.candidates_ready, 1);

    // Publication plus static validation is sufficient; no evidence PUT runs.
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["status"], "Active");
    assert_eq!(view["activeRevision"], 1);
    let revision = get_json(&app, "/api/v1/template-profiles/k8s-linux/revisions/1").await;
    assert_eq!(revision["state"], "Active");
    assert!(revision
        .get("reason")
        .is_some_and(serde_json::Value::is_null));
    let head = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    assert!(head
        .active_attestation_id
        .as_deref()
        .unwrap()
        .starts_with("static-validation-v1:"));
    control_plane
        .periodic_scan(1_800_000_001_100)
        .await
        .unwrap();
    let repeated = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repeated.active_attestation_id, head.active_attestation_id);
}

#[tokio::test]
async fn automatic_activation_allows_fleet_admission_and_exact_input_contract_without_attestation()
{
    let (app, control_plane, _engine) = build_app_with_scan().await;
    let digest =
        common::attestation_harness::seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let head = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    let activation = head.active_attestation_id.unwrap();
    assert!(activation.starts_with("static-validation-v1:"));
    let contract = get_json(
        &app,
        "/api/v1/template-profiles/k8s-linux/revisions/1/input-contract",
    )
    .await;
    assert_eq!(contract["profileKey"], "k8s-linux");
    assert_eq!(contract["revision"], 1);
    assert_eq!(contract["artifactDigest"], digest);

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
    assert_eq!(fleet["resolved"]["template"]["revision"], 1);
    assert_eq!(fleet["resolved"]["template"]["artifactDigest"], digest);
    assert_eq!(fleet["resolved"]["template"]["attestationId"], activation);
}

#[tokio::test]
async fn forged_subject_members_remain_unverified_without_changing_activation() {
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

    let activation = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap()
        .active_attestation_id;
    let expected = expected_bindings_digest("k8s-linux", 1);
    // The honest body, then single-member forgeries: protected bindings,
    // provider version, engine binary digest and suite. A subject the registry
    // cannot reproduce is recorded as MISMATCHED EVIDENCE (R6-08: 201 +
    // audited, subject_verified=false) without changing activation — and
    // each forgery carries its OWN attestation key, because the same
    // key with a different body would be a replay conflict instead.
    let mut forgeries: Vec<(String, String)> = Vec::new();
    forgeries.push((
        "bindings".into(),
        attest_body(&control_plane, &digest, "bd1_forged", &engine).await,
    ));
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
        let record = get_json(
            &app,
            &format!(
                "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/att-forged-{member}"
            ),
        )
        .await;
        assert_eq!(record["subjectVerified"], false);
        assert_eq!(record["result"], "passed");
        let head = control_plane
            .template_profile_get("k8s-linux")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(head.active_attestation_id, activation, "forged {member}");
    }
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(
        view["status"], "Active",
        "forged evidence cannot downgrade the active revision"
    );

    // Honest evidence remains independently accepted after the forgeries.
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
    let evidence = get_json(
        &app,
        "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/att-good",
    )
    .await;
    assert_eq!(evidence["subjectVerified"], true);
    let after = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.active_attestation_id, activation);
    assert_eq!(after.active_revision, Some(1));
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
