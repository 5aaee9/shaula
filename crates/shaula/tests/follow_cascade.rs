#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! spec 0023: bare-key follower fleets cascade to the latest Active
//! template revision on the scan cadence; exact pins never move; the
//! zero-occupancy replacement gate still applies to the cascade.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shaula_core::lifecycle::GenerationState;
use shaula_core::registry::{ControlPlaneStore, GenerationRecord, LifecycleStore};
use tower::ServiceExt;

use common::attestation_harness::seed_profile;
use common::*;

/// Same platform/shape as [`fixture_artifact`] with bumped policy marker —
/// a genuinely new artifact digest for the profile's second revision.
fn fixture_artifact_v2() -> (String, Vec<u8>) {
    fixture_artifact_with_policy("sha256:policy-v2")
}

fn fixture_artifact_with_policy(policy: &str) -> (String, Vec<u8>) {
    let manifest = format!(
        "api_version: shaula.io/template-profile/v1\nkind: RunnerTemplateProfile\nplatform: kubernetes\nruntime:\n  protocol: terraform-cli/v1\n  engine: terraform\n  root_module: .\n  required_version: \">= 1.9, < 2.0\"\nbindings_contract: shaula.bindings.kubernetes/v1\ncontainer_bootstrap_contract: shaula.container-bootstrap/v1\nschemas:\n  bindings: schemas/bindings.schema.json\n  parameters: schemas/parameters.schema.json\nmanaged_resource_shape:\n  - role: bootstrap\n    terraform_type: kubernetes_secret_v1\n    exact_count: 1\n  - role: runner\n    terraform_type: kubernetes_pod_v1\n    exact_count: 1\nrunner_image_digests:\n  - ghcr.io/actions/actions-runner:2.323.0@sha256:3f2a1b9c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8\nruntime_policy_digest: {policy}\n"
    );
    let mut builder = tar::Builder::new(Vec::new());
    for (name, content) in [
        ("profile.yaml", manifest.as_str()),
        (".terraform.lock.hcl", FIXTURE_LOCK_HCL),
        ("schemas/bindings.schema.json", "{}"),
        ("schemas/parameters.schema.json", "{}"),
        (
            "main.tf",
            "resource \"kubernetes_secret_v1\" \"bootstrap\" {}\n",
        ),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, content.as_bytes())
            .unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
    let bytes = encoder.finish().unwrap();
    let digest = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };
    (digest, bytes)
}

/// Publishes the profile's revision 2 (new artifact, optional policy
/// replacement) and drives the scan to activate it.
async fn publish_revision_2(
    app: &axum::Router,
    control_plane: &std::sync::Arc<shaula_store::registry_impl::SqliteControlPlane>,
    policy_json: Option<&str>,
) -> String {
    let (digest2, bytes2) = fixture_artifact_v2();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{digest2}"))
        .header("authorization", common::oidc::bearer("template.publish"))
        .body(Body::from(bytes2))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "artifact upload failed: {:?}",
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    );
    let body = match policy_json {
        Some(policy) => TEMPLATE_PUT_BODY
            .replace("PLACEHOLDER", &digest2)
            .replace(r#"{"size_class": ["standard"]}"#, policy),
        None => TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &digest2),
    };
    let current = app
        .clone()
        .oneshot(authorized(
            "GET",
            "/api/v1/template-profiles/k8s-linux",
            None,
        ))
        .await
        .unwrap();
    let etag = current.headers().get("etag").unwrap().clone();
    let request = Request::builder()
        .method("PUT")
        .uri("/api/v1/template-profiles/k8s-linux")
        .header(
            "authorization",
            common::oidc::bearer("template.read template.publish template.attest"),
        )
        .header("if-match", etag)
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    if status != StatusCode::ACCEPTED {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        panic!(
            "profile PUT failed {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    control_plane
        .periodic_scan(1_800_000_002_000)
        .await
        .unwrap();
    let head = control_plane
        .template_profile_get("k8s-linux")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.active_revision, Some(2));
    digest2
}

async fn fleet_revision_pin(app: &axum::Router, fleet: &str) -> (i64, i64) {
    let view = get_json(app, &format!("/api/v1/fleets/{fleet}")).await;
    (
        view["metadata"]["revision"].as_i64().unwrap(),
        view["resolved"]["template"]["revision"].as_i64().unwrap(),
    )
}

#[tokio::test]
async fn follower_auto_upgrades_after_new_revision_activates() {
    let (app, control_plane, _engine, service) = build_app_with_service().await;
    seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let create = put_with_idempotency("/api/v1/fleets/linux-x64", "follow-1", FLEET_BODY.into());
    assert_eq!(
        app.clone().oneshot(create).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (1, 1));

    let digest2 = publish_revision_2(&app, &control_plane, None).await;
    // The cascade is level-triggered on the scan cadence, not on publish.
    let upgraded = service
        .cascade_template_follow_upgrades(1_800_000_003_000)
        .await
        .unwrap();
    assert_eq!(upgraded, 1);
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (2, 2));
    let fleet = get_json(&app, "/api/v1/fleets/linux-x64").await;
    assert_eq!(
        fleet["resolved"]["template"]["artifactDigest"],
        digest2.as_str()
    );

    // No lag remains: a second pass mints nothing.
    let again = service
        .cascade_template_follow_upgrades(1_800_000_004_000)
        .await
        .unwrap();
    assert_eq!(again, 0);
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (2, 2));
}

#[tokio::test]
async fn pinned_fleet_never_cascades() {
    let (app, control_plane, _engine, service) = build_app_with_service().await;
    seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let pinned = FLEET_BODY.replace(
        r#""template_profile_ref": "k8s-linux""#,
        r#""template_profile_ref": {"key": "k8s-linux", "revision": 1}"#,
    );
    let create = put_with_idempotency("/api/v1/fleets/linux-x64", "pin-1", pinned);
    assert_eq!(
        app.clone().oneshot(create).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    publish_revision_2(&app, &control_plane, None).await;
    let upgraded = service
        .cascade_template_follow_upgrades(1_800_000_003_000)
        .await
        .unwrap();
    assert_eq!(upgraded, 0);
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (1, 1));
}

#[tokio::test]
async fn occupied_follower_defers_then_upgrades_after_drain() {
    let (app, control_plane, _engine, service) = build_app_with_service().await;
    let digest1 = seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let create = put_with_idempotency("/api/v1/fleets/linux-x64", "occ-1", FLEET_BODY.into());
    assert_eq!(
        app.clone().oneshot(create).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    let revision = control_plane
        .fleet_revision_latest("linux-x64")
        .await
        .unwrap()
        .unwrap();
    let now = 1_800_000_002_500i64;
    control_plane
        .generation_insert(GenerationRecord {
            id: "gen-occupied".into(),
            fleet_key: "linux-x64".into(),
            runner_name: "runner-occupied".into(),
            generation_name: "occupied".into(),
            fleet_revision: 1,
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: digest1,
            attestation_id: "static-validation-v1:test".into(),
            inputs_digest: revision.inputs_digest.clone(),
            state: GenerationState::Idle,
            github_runner_id: None,
            workspace_path: "/tmp/unused".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    publish_revision_2(&app, &control_plane, None).await;
    // Occupancy blocks the replacement gate — the cascade defers.
    let upgraded = service
        .cascade_template_follow_upgrades(1_800_000_003_000)
        .await
        .unwrap();
    assert_eq!(upgraded, 0);
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (1, 1));

    // Drain along the legal lifecycle path (insert always lands as
    // CreatePending at the persistence boundary).
    for state in [
        GenerationState::Creating,
        GenerationState::WaitingOnline,
        GenerationState::Idle,
        GenerationState::Retiring,
        GenerationState::DestroyPending,
        GenerationState::Destroying,
        GenerationState::Destroyed,
    ] {
        control_plane
            .generation_advance("gen-occupied", state, now)
            .await
            .unwrap();
    }
    let upgraded = service
        .cascade_template_follow_upgrades(1_800_000_005_000)
        .await
        .unwrap();
    assert_eq!(upgraded, 1);
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (2, 2));
}

#[tokio::test]
async fn follower_with_inputs_rejected_by_new_policy_skips_upgrade() {
    let (app, control_plane, _engine, service) = build_app_with_service().await;
    seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let with_inputs = FLEET_BODY.replace(
        r#""template_inputs": {}"#,
        r#""template_inputs": {"size_class": "standard"}"#,
    );
    let create = put_with_idempotency("/api/v1/fleets/linux-x64", "pol-1", with_inputs);
    assert_eq!(
        app.clone().oneshot(create).await.unwrap().status(),
        StatusCode::ACCEPTED
    );

    // Revision 2 narrows the allowed alias set, excluding the fleet's input.
    publish_revision_2(
        &app,
        &control_plane,
        Some(r#"{"size_class": ["enterprise"]}"#),
    )
    .await;
    let upgraded = service
        .cascade_template_follow_upgrades(1_800_000_003_000)
        .await
        .unwrap();
    assert_eq!(upgraded, 0);
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (1, 1));
}
