#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Attestation REQUEST-BOUNDARY semantics (R7-03 / R7-04 / R7-05): a
//! missing published authority is a retryable storage fault that never
//! consumes the attestation key; non-canonical subjects are rejected by
//! the strict parse and never persisted; and control bytes in URI keys
//! are refused before they can enter durable identity material.

mod common;

use axum::http::StatusCode;
use tower::ServiceExt;

use common::attestation_harness::{
    put_attestation, put_attestation_profile, put_template_profile_raw, seed_profile,
};

#[tokio::test]
async fn template_profile_conditional_writes_are_enforced() {
    // R9-02 (spec 0005 §3): the desired head only moves through a valid
    // conditional write — no preconditions is 428, a stale ETag is 412,
    // `If-None-Match: *` on an existing Profile is 412, and identical
    // content with a FRESH idempotency key is a durable NoOp 200 that
    // mints no revision.
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest);

    // A PUT with NO conditional headers is 428 — never an unconditional
    // overwrite.
    let status = put_template_profile_raw(&app, "k8s-linux", body.clone(), false, None, None).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);

    // A stale If-Match can never advance the head.
    let status = put_template_profile_raw(
        &app,
        "k8s-linux",
        body.clone(),
        false,
        Some("\"bogus:9\"".to_string()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);

    // The CURRENT ETag from the profile view drives the remaining cases.
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    let etag = format!(
        "\"{}:{}\"",
        view["incarnation"].as_str().unwrap(),
        view["desiredRevision"].as_i64().unwrap()
    );

    // Correct If-Match + identical content + fresh idempotency key →
    // durable NoOp 200, NO new revision.
    let status = put_template_profile_raw(
        &app,
        "k8s-linux",
        body.clone(),
        false,
        Some(etag.clone()),
        Some("fresh-idem-key".to_string()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "identical content is a durable re-assertion, not a new revision"
    );
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["desiredRevision"], 1, "no revision 2 may be minted");

    // Changed content under the same correct If-Match advances the head.
    let body2 = template_put_body_r2(&digest);
    let status = put_template_profile_raw(&app, "k8s-linux", body2, false, Some(etag), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "changed content advances");
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["desiredRevision"], 2);
    drop(engine);
}
use common::*;

#[tokio::test]
async fn unreadable_published_authority_is_retryable_never_a_recorded_mismatch() {
    use shaula_core::registry::ControlPlaneStore;

    // R7-03: without the published dependency lock the registry cannot
    // conclude ANYTHING about the subject. The failure must surface as a
    // retryable server error, must NOT consume the attestation key with
    // a recorded lie, and the SAME request must succeed after the file
    // is restored.
    let (app, control_plane, engine, artifact_root) = build_app_with_artifact_root().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    let expected = expected_bindings_digest("k8s-linux", 1);
    let attest = attest_body(&control_plane, &digest, &expected, &engine).await;

    let lock_path = shaula_core::artifact_layout::artifact_dir(&artifact_root, &digest)
        .unwrap()
        .join(".terraform.lock.hcl");
    let backup_path = lock_path.with_extension("hcl.bak");
    std::fs::rename(&lock_path, &backup_path).unwrap();

    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-io", attest.clone()).await;
    assert!(
        status.is_server_error(),
        "a missing authority is a storage fault, not a verdict: {status}"
    );
    assert!(
        control_plane
            .attestation_get("k8s-linux", 1, "att-io")
            .await
            .unwrap()
            .is_none(),
        "the key must stay uncommitted — no evidence without an authority"
    );

    // The operator restores the published file; the SAME request now
    // verifies and activates.
    std::fs::rename(&backup_path, &lock_path).unwrap();
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-io", attest).await;
    assert_eq!(status, StatusCode::CREATED, "the retry must succeed");
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 1, "the retry must activate");
}

#[tokio::test]
async fn control_bytes_in_uri_keys_are_rejected_at_the_boundary() {
    use shaula_core::registry::ControlPlaneStore;

    // R7-05: keys that are not legitimate URI path segments (NUL bytes)
    // never enter durable identity material — neither as a profile key
    // nor as an attestation key.
    let (app, control_plane, engine) = build_app_with_scan().await;

    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", "sha256:00");
    let (status, _) = put_attestation_profile(&app, "a%001", body).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a NUL-bearing profile key is not a stable identifier"
    );

    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let expected = expected_bindings_digest("k8s-linux", 1);
    let attest = attest_body(&control_plane, &digest, &expected, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "2%00x", attest).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a NUL-bearing attestation key is not a stable identifier"
    );
    assert!(
        control_plane
            .attestation_get("k8s-linux", 1, "2\x00x")
            .await
            .unwrap()
            .is_none(),
        "rejected keys commit nothing"
    );
}

#[tokio::test]
async fn whitespace_normalized_keys_are_rejected_not_silently_rewritten() {
    use shaula_core::registry::ControlPlaneStore;

    // R8-01: the key validator no longer TRIMS — a key that whitespace
    // normalization would rewrite is rejected at the boundary, so the
    // validated identity is always EXACTLY the persisted identity. The
    // pre-R8 behavior stored "\na" while fleet references resolved "a"
    // — a dangling durable reference.
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    // A padded variant of the SAME key never reaches publication: the
    // boundary refuses it before any lookup or commit.
    let body = TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest);
    let (status, _) = put_attestation_profile(&app, "%0Ak8s-linux", body.clone()).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a newline-padded template profile key is not a stable identifier"
    );
    let (status, _) = put_attestation_profile(&app, "k8s-linux%09", body).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a tab-padded template profile key is not a stable identifier"
    );

    // The same strictness governs Auth Profile publication.
    let (status, _) = put_auth_profile(&app, "%0Aprod-app").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a newline-padded auth profile key is not a stable identifier"
    );
    // Nothing was stored under a rewritten identity.
    assert!(control_plane
        .attestation_get("\nk8s-linux", 1, "x")
        .await
        .unwrap()
        .is_none(),);
    drop(engine);
}

async fn put_auth_profile(app: &axum::Router, key: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(authorized(
            "PUT",
            &format!("/api/v1/github-auth-profiles/{key}"),
            Some(AUTH_PUT_BODY.into()),
        ))
        .await
        .unwrap();
    let status = response.status();
    let text = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&text).into_owned())
}

#[tokio::test]
async fn corrupt_published_manifest_is_retryable_never_a_recorded_mismatch() {
    use shaula_core::registry::ControlPlaneStore;

    // R8-02: a READABLE but invalid published manifest is server-side
    // authority corruption — the registry cannot recompute the expected
    // subject, so it must answer with a retryable storage error and keep
    // the key uncommitted, never record the client's subject as
    // "mismatched" under a broken authority.
    let (app, control_plane, engine, artifact_root) = build_app_with_artifact_root().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    let expected = expected_bindings_digest("k8s-linux", 1);
    let attest = attest_body(&control_plane, &digest, &expected, &engine).await;

    let manifest_path = shaula_core::artifact_layout::artifact_dir(&artifact_root, &digest)
        .unwrap()
        .join("profile.yaml");
    let original = std::fs::read_to_string(&manifest_path).unwrap();
    std::fs::write(&manifest_path, "{ not: [valid").unwrap();

    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-corrupt", attest.clone()).await;
    assert!(
        status.is_server_error(),
        "corrupt authority is a storage fault, not a verdict: {status}"
    );
    assert!(
        control_plane
            .attestation_get("k8s-linux", 1, "att-corrupt")
            .await
            .unwrap()
            .is_none(),
        "the key must stay uncommitted — no mismatch fact without an authority"
    );

    // Restoring the published manifest makes the SAME request succeed.
    std::fs::write(&manifest_path, original).unwrap();
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-corrupt", attest).await;
    assert_eq!(status, StatusCode::CREATED, "the retry must succeed");
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 1, "the retry must activate");
}

#[tokio::test]
async fn non_canonical_subject_is_a_422_and_never_reaches_the_durable_record() {
    use shaula_core::registry::ControlPlaneStore;

    // R7-04: the subject is a TYPED canonical document. Payload the spec
    // excludes (bindings, raw test output) — or missing canonical
    // members — is rejected by the strict parse at the request boundary;
    // arbitrary request JSON is never persisted as mismatch evidence.
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    let raw_subject = r#"{
        "result": "passed",
        "subject": {
            "bindings": {"kubeconfig": "SYNTHETIC-NOT-A-SECRET"},
            "raw_test_output": "SYNTHETIC"
        },
        "suite": {"name": "shaula-template-conformance", "version": "v1"},
        "completed_at": 1800000000
    }"#;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-raw", raw_subject.into()).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a non-canonical subject is an invalid request"
    );
    assert!(
        control_plane
            .attestation_get("k8s-linux", 1, "att-raw")
            .await
            .unwrap()
            .is_none(),
        "nothing — least of all raw bindings or output — may be stored"
    );

    // A subject that ALSO carries an unknown extra field on top of the
    // canonical members is equally rejected (strict, not lenient).
    let expected = expected_bindings_digest("k8s-linux", 1);
    let honest = attest_body(&control_plane, &digest, &expected, &engine).await;
    let mut extra: serde_json::Value = serde_json::from_str(&honest).unwrap();
    extra["subject"]["extra_field"] = serde_json::json!("SYNTHETIC");
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-extra", extra.to_string()).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown subject members are denied, never silently stored"
    );
}
