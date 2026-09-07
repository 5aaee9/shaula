//! Attestation replay/freeze semantics at the HTTP boundary (R6-01 /
//! R6-06 / R6-07 / R6-08): the attestation identity is the FULL path
//! scope (Profile, Revision, key); an exact replay returns the original
//! record BEFORE the current engine authority is consulted; the freeze
//! is one-time PER REVISION so a new desired Ready candidate can
//! promote; and refused activations stay durable+audited. The R7-03/04/05
//! request-boundary semantics live in convergence_attestation_boundary.rs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::http::StatusCode;

use common::attestation_harness::{put_attestation, put_template_profile, seed_profile};
use common::*;

#[tokio::test]
async fn identical_attestation_replay_never_overwrites_the_frozen_active_id() {
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    // First attestation activates and freezes the active id (R5-09).
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

    // A different key on the ALREADY-ACTIVE revision is durable
    // evidence that cannot replace the frozen attestation (R6-08: 201,
    // RecordedNotActivated; the one-time-per-revision freeze holds).
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
}

#[tokio::test]
async fn a_new_desired_ready_revision_can_promote_after_r1_was_active() {
    // R6-01: the freeze is one-time PER REVISION — publishing r2,
    // scanning it to Ready and attesting it with a fresh key must
    // PROMOTE it over r1 instead of conflicting forever.
    let (app, control_plane, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    let expected1 = expected_bindings_digest("k8s-linux", 1);
    let attest1 = attest_body(&control_plane, &digest, &expected1, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "ci-r1", attest1).await;
    assert_eq!(status, StatusCode::CREATED);
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 1);

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

    // A new attestation with a fresh key activates r2 (upgrade path).
    let expected2 = expected_bindings_digest("k8s-linux", 2);
    let attest2 = attest_body(&control_plane, &digest, &expected2, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 2, "ci-r2", attest2).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the new desired Ready candidate must be able to promote"
    );
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 2, "r2 promotes over r1");
    assert_eq!(view["status"], "Active");
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

    // A LATE, correct r1 attestation (fresh key): recorded evidence,
    // but r1 is no longer desired — activation refused, revision stays.
    let expected1 = expected_bindings_digest("k8s-linux", 1);
    let late = attest_body(&control_plane, &digest, &expected1, &engine).await;
    let (status, _) = put_attestation(&app, "k8s-linux", 1, "ci-r1-late", late).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "stale but correct evidence stays durable (R6-08)"
    );
    let view = get_json(&app, "/api/v1/template-profiles/k8s-linux").await;
    assert_eq!(view["activeRevision"], 1);
    assert_eq!(view["desiredRevision"], 2);
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
