//! Conformance evidence remains immutable and independent of automatic
//! activation: exact replay precedes current authority checks, and neither
//! new evidence nor replay can overwrite frozen activation provenance.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::http::StatusCode;
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

use common::attestation_harness::{put_attestation, put_template_profile, seed_profile};
use common::*;

#[tokio::test]
async fn identical_attestation_replay_never_overwrites_the_frozen_active_id() {
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    let original = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    // Static validation already froze activation; conformance is independent.
    let expected = expected_bindings_digest("k8s-linux", 1);
    let attest = attest_body(&control_plane, &digest, &expected, &engine).await;
    let (status, replay) = put_attestation(&app, "k8s-linux", 1, "att-1", attest.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(replay.contains("att-1"));
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["status"], "Active");
    assert_eq!(view["activeRevision"], 1);

    // EXACT replay (same URI, same body): the ORIGINAL record stands —
    // no new id, no re-activation, no overwrite.
    let (status, replay) = put_attestation(&app, "k8s-linux", 1, "att-1", attest.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        replay.contains("att-1"),
        "exact replay must answer with the original stable attestation id"
    );

    // Same key with a DIFFERENT canonical body conflicts (409).
    let conflicting = attest.replace("1800000000", "1800000001");
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-1", conflicting).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Another evidence key cannot replace the frozen activation provenance.
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-2", attest).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "cannot-activate is not cannot-record"
    );

    // The profile still shows exactly the first activation.
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["status"], "Active");
    assert_eq!(view["activeRevision"], 1);
    let current = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current.active_attestation_id,
        original.active_attestation_id
    );
}

#[tokio::test]
async fn automatic_promotion_keeps_existing_fleet_pin_and_input_contract() {
    let (app, control_plane, _engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 1);
    let created = app
        .clone()
        .oneshot(put_with_idempotency(
            "/api/v1/fleets/linux-x64",
            "automatic-r1",
            FLEET_BODY.into(),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::ACCEPTED);
    let etag = created.headers()["etag"].clone();
    let original = get_json(&app, "/api/v1/fleets/linux-x64").await;
    let original_contract = get_json(
        &app,
        "/api/v1/template-profiles/k8s-linux/revisions/1/input-contract",
    )
    .await;

    // Publish r2 (same artifact, CHANGED input policy): identical
    // content would be a durable NoOp under R9-02, so a genuine r2
    // must change something.
    let body = template_put_body_r2(&digest);
    assert_eq!(
        put_template_profile(&app, "k8s-linux", body).await,
        StatusCode::ACCEPTED
    );
    control_plane
        .periodic_scan(1_800_000_002_000)
        .await
        .unwrap();

    // No conformance requests are needed for either revision's activation.
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 2, "r2 promotes over r1");
    assert_eq!(view["status"], "Active");
    let retained = get_json(&app, "/api/v1/fleets/linux-x64").await;
    assert_eq!(retained["resolved"], original["resolved"]);
    assert_eq!(retained["resolved"]["template"]["revision"], 1);
    assert_eq!(
        get_json(
            &app,
            "/api/v1/template-profiles/k8s-linux/revisions/1/input-contract"
        )
        .await,
        original_contract
    );
    let mut noop = authorized("PUT", "/api/v1/fleets/linux-x64", Some(FLEET_BODY.into()));
    noop.headers_mut().remove("if-none-match");
    noop.headers_mut().insert("if-match", etag.clone());
    let response = app.clone().oneshot(noop).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["etag"], etag);
    assert_eq!(
        get_json(&app, "/api/v1/fleets/linux-x64").await["resolved"],
        original["resolved"]
    );
}

#[tokio::test]
async fn stale_attestation_stays_durable_and_cannot_promote() {
    // R6-08: a CORRECT attestation that arrives for a superseded
    // revision (desired has moved on) is recorded and audited but never
    // replaces the promoted revision.
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    let expected1 = expected_bindings_digest("k8s-linux", 1);
    let attest1 = attest_body(&control_plane, &digest, &expected1, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "ci-r1", attest1).await;
    assert_eq!(status, StatusCode::CREATED);

    // Advance the desired head to r2 (changed input policy — identical
    // content would be a durable NoOp under R9-02).
    let body = template_put_body_r2(&digest);
    assert_eq!(
        put_template_profile(&app, "k8s-linux", body).await,
        StatusCode::ACCEPTED
    );
    control_plane
        .periodic_scan(1_800_000_002_000)
        .await
        .unwrap();
    let promoted = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();

    // A LATE, correct r1 attestation (fresh key): recorded evidence,
    // but r1 is no longer desired and the active r2 identity cannot change.
    let expected1 = expected_bindings_digest("k8s-linux", 1);
    let late = attest_body(&control_plane, &digest, &expected1, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "ci-r1-late", late).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "stale but correct evidence stays durable (R6-08)"
    );
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 2);
    assert_eq!(view["desiredRevision"], 2);
    let evidence = get_json(
        &app,
        "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/ci-r1-late",
    )
    .await;
    assert_eq!(evidence["subjectVerified"], true);
    let after = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.active_attestation_id, promoted.active_attestation_id);
}

#[tokio::test]
async fn failed_conformance_remains_readable_without_downgrading_activation() {
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let original = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    let expected = expected_bindings_digest("k8s-linux", 1);
    let mut body: serde_json::Value =
        serde_json::from_str(&attest_body(&control_plane, &digest, &expected, &engine).await)
            .unwrap();
    body["result"] = serde_json::json!("failed");
    let (status, _) =
        put_attestation(&app, "k8s-linux", 1, "failed-runtime", body.to_string()).await;
    assert_eq!(status, StatusCode::CREATED);
    let evidence = get_json(
        &app,
        "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/failed-runtime",
    )
    .await;
    assert_eq!(evidence["result"], "failed");
    assert_eq!(evidence["subjectVerified"], true);
    let after = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.active_revision, Some(1));
    assert_eq!(after.active_attestation_id, original.active_attestation_id);
    assert_eq!(after.status, "Active");
}

#[tokio::test]
async fn same_attestation_key_in_another_profile_never_conflicts() {
    // R6-06: the URI key is scoped by Profile and Revision — unrelated
    // profiles reusing the same CI label must not collide.
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let digest_b = seed_profile(&app, &control_plane, "k8s-linux-2", false).await;
    assert_eq!(digest, digest_b, "same artifact fixture re-published");

    let expected_a = expected_bindings_digest("k8s-linux", 1);
    let body_a = attest_body(&control_plane, &digest, &expected_a, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "ci", body_a).await;
    assert_eq!(status, StatusCode::CREATED);

    let expected_b = expected_bindings_digest("k8s-linux-2", 1);
    let body_b = attest_body(&control_plane, &digest, &expected_b, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux-2", 1, "ci", body_b).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the same URI key under ANOTHER profile is a distinct identity"
    );
}

#[tokio::test]
async fn replay_reaches_the_original_record_even_after_the_engine_changed() {
    // R6-07: an exact replay is answered from the persisted record by
    // the PRE-CHECK — replacing the engine binary afterwards must not
    // turn a historically accepted request into a 422.
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    let expected = expected_bindings_digest("k8s-linux", 1);
    let attest = attest_body(&control_plane, &digest, &expected, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "att-1", attest.clone()).await;
    assert_eq!(status, StatusCode::CREATED);

    // The configured engine file mutates (operator replaced the binary).
    std::fs::write(&engine, b"@echo off\r\nexit /b 1\r\n").unwrap();

    // The historical replay still returns the original 201.
    let (status, body) = put_attestation(&app, "k8s-linux", 1, "att-1", attest).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "replay must be served from the durable record, not the current authority"
    );
    assert!(body.contains("att-1"));
}
