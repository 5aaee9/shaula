//! Spec 0028: operator finalization of Quarantined generations —
//! ledger-only `Quarantined -> Destroyed` through
//! `POST /api/v1/generations/{id}/finalize` with `fleet.retire` scope,
//! a mandatory reason, replay-safe idempotency and audit evidence.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::registry::{GenerationRecord, LifecycleStore};
use tower::ServiceExt;

use common::attestation_harness::seed_profile;
use common::*;

async fn seed_quarantined(store: &impl LifecycleStore, id: &str, digest: &str) {
    store
        .generation_insert(GenerationRecord {
            id: id.into(),
            fleet_key: "f1".into(),
            runner_name: format!("runner-{id}"),
            generation_name: format!("generation-{id}"),
            fleet_revision: 1,
            pool_member_key: None,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest.into(),
            attestation_id: "att".into(),
            inputs_digest: "inputs".into(),
            state: G::CreatePending,
            github_runner_id: None,
            workspace_path: "unused".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    store
        .generation_advance(id, G::Quarantined, 2)
        .await
        .unwrap();
}

fn finalize_request(id: &str, reason: &str, idempotency: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/generations/{id}/finalize"))
        .header("authorization", common::oidc::bearer("fleet.retire"))
        .header("content-type", "application/json");
    if let Some(key) = idempotency {
        builder = builder.header("idempotency-key", key);
    }
    builder
        .body(Body::from(format!("{{\"reason\":{}}}", json_str(reason))))
        .unwrap()
}

fn json_str(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

#[tokio::test]
async fn finalize_quarantined_generation_terminates_and_replays() {
    let (app, store, _engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &store, "k8s-linux", true).await;
    // Fleet must exist for the generation's fleet_key to be meaningful.
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/fleets/f1",
                Some(FLEET_BODY.into())
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    seed_quarantined(store.as_ref(), "gen-q1", &digest).await;

    // Missing scope → 403-equivalent problem.
    let denied = Request::builder()
        .method("POST")
        .uri("/api/v1/generations/gen-q1/finalize")
        .header("authorization", common::oidc::bearer("fleet.read"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"reason":"verified gone"}"#))
        .unwrap();
    let denied = app.clone().oneshot(denied).await.unwrap();
    assert!(
        matches!(
            denied.status(),
            StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY
        ),
        "fleet.read must not finalize: {}",
        denied.status()
    );

    // Blank reason → 422.
    let blank = finalize_request("gen-q1", "   ", None);
    assert_eq!(
        app.clone().oneshot(blank).await.unwrap().status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Happy path: Quarantined → Destroyed.
    let accepted = finalize_request("gen-q1", "verified VM removed on pve", Some("fin-1"));
    let response = app.clone().oneshot(accepted).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "finalize must accept: {:?}",
        axum::body::to_bytes(response.into_body(), usize::MAX).await
    );
    let generation = store.generation_get("gen-q1").await.unwrap().unwrap();
    assert_eq!(generation.state, G::Destroyed);
    assert!(!generation.state.counts_occupancy());

    // Exact replay with the same idempotency key returns the recorded result.
    let replay = finalize_request("gen-q1", "verified VM removed on pve", Some("fin-1"));
    assert_eq!(
        app.clone().oneshot(replay).await.unwrap().status(),
        StatusCode::ACCEPTED
    );

    // Same key, different reason → idempotency conflict.
    let conflict = finalize_request("gen-q1", "different claim", Some("fin-1"));
    assert_eq!(
        app.clone().oneshot(conflict).await.unwrap().status(),
        StatusCode::CONFLICT
    );

    // A second finalize on the already-Destroyed generation → 409,
    // never a silent no-op.
    let again = finalize_request("gen-q1", "verified again", Some("fin-2"));
    assert_eq!(
        app.clone().oneshot(again).await.unwrap().status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn finalize_rejects_missing_or_non_quarantined() {
    let (app, store, _engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &store, "k8s-linux", true).await;
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/fleets/f1",
                Some(FLEET_BODY.into())
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );

    // Unknown id → 404.
    let missing = finalize_request("gen-absent", "verified", None);
    assert_eq!(
        app.clone().oneshot(missing).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );

    // An Idle generation cannot be finalized — finalize is not a
    // kill switch for live runners.
    store
        .generation_insert(GenerationRecord {
            id: "gen-idle".into(),
            fleet_key: "f1".into(),
            runner_name: "runner-idle".into(),
            generation_name: "generation-idle".into(),
            fleet_revision: 1,
            pool_member_key: None,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest,
            attestation_id: "att".into(),
            inputs_digest: "inputs".into(),
            state: G::CreatePending,
            github_runner_id: None,
            workspace_path: "unused".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    for state in [G::Creating, G::WaitingOnline, G::Idle] {
        store
            .generation_advance("gen-idle", state, 2)
            .await
            .unwrap();
    }
    let live = finalize_request("gen-idle", "not quarantined", None);
    assert_eq!(
        app.clone().oneshot(live).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        store
            .generation_get("gen-idle")
            .await
            .unwrap()
            .unwrap()
            .state,
        G::Idle
    );
}
