//! Fleet label updates keep the resource identity and use ordinary revision CAS.

// Shared HTTP fixture helpers deliberately fail fast on setup errors.
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderValue, Request, StatusCode};
use shaula_core::lifecycle::GenerationState;
use shaula_core::registry::{ControlPlaneStore, GenerationRecord, LifecycleStore};
use tower::ServiceExt;

use common::attestation_harness::{put_attestation, seed_profile};
use common::*;

#[tokio::test]
async fn fleet_labels_can_change_after_creation() -> Result<(), Box<dyn std::error::Error>> {
    let (app, store, engine) = build_app_with_scan().await;
    let digest = seed_profile(&app, &store, "k8s-linux", true).await;
    let attestation = attest_body(
        &store,
        &digest,
        &expected_bindings_digest("k8s-linux", 1),
        &engine,
    )
    .await;
    assert_eq!(
        put_attestation(&app, "k8s-linux", 1, "att-1", attestation)
            .await
            .0,
        StatusCode::CREATED
    );

    let path = "/api/v1/fleets/linux-x64";
    let response = app
        .clone()
        .oneshot(authorized("PUT", path, Some(FLEET_BODY.into())))
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let original_etag = response
        .headers()
        .get("etag")
        .ok_or("missing ETag")?
        .clone();
    let original_head = store.fleet_get("linux-x64").await?.ok_or("missing Fleet")?;
    let original_revision = store
        .fleet_revision_latest("linux-x64")
        .await?
        .ok_or("missing revision")?;

    // Existing work must not turn a routing-only edit into a replacement gate.
    store
        .generation_insert(GenerationRecord {
            id: "busy-generation".into(),
            fleet_key: "linux-x64".into(),
            runner_name: "busy-runner".into(),
            generation_name: "busy-generation".into(),
            fleet_revision: 1,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest,
            attestation_id: "att-1".into(),
            inputs_digest: original_revision.inputs_digest.clone(),
            state: GenerationState::CreatePending,
            github_runner_id: None,
            workspace_path: "retained-workspace".into(),
            created_at: 1,
            updated_at: 1,
        })
        .await?;
    store
        .generation_set_github_runner("busy-generation", 12, 2)
        .await?;
    for state in [
        GenerationState::Creating,
        GenerationState::WaitingOnline,
        GenerationState::Idle,
        GenerationState::Busy,
    ] {
        store
            .generation_advance("busy-generation", state, 2)
            .await?;
    }
    assert_eq!(store.generations_occupancy("linux-x64").await?, 1);

    let mut updated: serde_json::Value = serde_json::from_str(FLEET_BODY)?;
    updated["github"]["labels"] = serde_json::json!(["linux", "x64", "build"]);
    let body = serde_json::to_string(&updated)?;
    let response = app
        .clone()
        .oneshot(replace(path, &original_etag, &body)?)
        .await?;
    let status = response.status();
    let updated_etag = response.headers().get("etag").cloned();
    let response_body = to_bytes(response.into_body(), 1 << 20).await?;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "label-only replacement must be accepted: {}",
        String::from_utf8_lossy(&response_body)
    );

    let head = store.fleet_get("linux-x64").await?.ok_or("missing Fleet")?;
    let revision = store
        .fleet_revision_latest("linux-x64")
        .await?
        .ok_or("missing revision")?;
    assert_eq!(head.incarnation, original_head.incarnation);
    assert_eq!(head.desired_revision, 2);
    assert_eq!(head.mutation_fence, original_head.mutation_fence + 1);
    assert_eq!(revision.auth_desired, original_revision.auth_desired);
    assert_eq!(
        revision.template_revision,
        original_revision.template_revision
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&revision.spec_json)?,
        updated
    );
    assert_eq!(store.generations_occupancy("linux-x64").await?, 1);
    let generation = store
        .generation_get("busy-generation")
        .await?
        .ok_or("missing generation")?;
    assert_eq!(generation.state, GenerationState::Busy);
    assert_eq!(generation.fleet_revision, 1);
    assert_eq!(generation.github_runner_id, Some(12));
    let updated_etag = updated_etag.ok_or("missing updated ETag")?;
    for (pointer, value) in [
        (
            "/github/target",
            serde_json::json!({"kind":"organization","owner":"EXAMPLE-ORG"}),
        ),
        (
            "/github/scale_set_name",
            serde_json::json!("different-name"),
        ),
        ("/github/runner_group", serde_json::json!("different-group")),
    ] {
        let mut changed_identity = updated.clone();
        *changed_identity
            .pointer_mut(pointer)
            .ok_or("missing identity field")? = value;
        let mut request = replace(
            path,
            &updated_etag,
            &serde_json::to_string(&changed_identity)?,
        )?;
        request.headers_mut().remove("idempotency-key");
        let response = app.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let problem = to_bytes(response.into_body(), 1 << 20).await?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&problem)?["detail"],
            "immutable identity change rejected"
        );
    }

    // A lost response replays the exact accepted revision despite the old ETag.
    let response = app
        .clone()
        .oneshot(replace(path, &original_etag, &body)?)
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        to_bytes(response.into_body(), 1 << 20).await?,
        response_body
    );
    let mut stale = replace(path, &original_etag, &body)?;
    stale.headers_mut().remove("idempotency-key");
    assert_eq!(
        app.oneshot(stale).await?.status(),
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(
        store
            .fleet_get("linux-x64")
            .await?
            .ok_or("missing Fleet")?
            .desired_revision,
        2
    );
    Ok(())
}

fn replace(path: &str, etag: &HeaderValue, body: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("PUT")
        .uri(path)
        .header("authorization", oidc::bearer("fleet.read fleet.write"))
        .header("if-match", etag)
        .header("idempotency-key", "replace-labels")
        .body(Body::from(body.to_owned()))
}
