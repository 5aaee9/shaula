#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! spec 0037: shared TemplatePool resources. Pool PUT admission resolves
//! member bare keys to the profiles' current Active revisions; deletion
//! is blocked while a live fleet references the pool; the two-stage
//! cascade re-resolves pool members on template activation and catches
//! referencing fleets up on their own zero-occupancy boundary.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::attestation_harness::{put_template_profile, seed_profile};
use common::*;
use shaula_core::registry::{ControlPlaneStore, LifecycleStore};

const POOL_BODY: &str = r#"{
    "members": [
        {"key": "primary", "template_profile_ref": "k8s-linux", "weight": 20, "template_inputs": {}},
        {"key": "secondary", "template_profile_ref": "k8s-linux", "weight": 10, "template_inputs": {}}
    ],
    "failure_policy": "backpressure"
}"#;

fn fleet_body_with_pool_ref() -> String {
    FLEET_BODY
        .replace(r#""template_profile_ref": "k8s-linux","#, "")
        .replace(
            r#""template_inputs": {}"#,
            r#""template_pool_ref": "builders""#,
        )
}

/// A capacity-only edit of the pool-referencing fleet: identical except
/// `max_runners` is bumped. Nothing about the template routing changes.
fn fleet_body_with_pool_ref_bigger() -> String {
    fleet_body_with_pool_ref().replace(r#""max_runners": 5"#, r#""max_runners": 10"#)
}

async fn pool_etag(app: &axum::Router, key: &str) -> String {
    let response = app
        .clone()
        .oneshot(authorized(
            "GET",
            &format!("/api/v1/template-pools/{key}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .trim_matches('"')
        .to_string()
}

/// Same shape as [`fixture_artifact`] with a distinct policy marker — a
/// genuinely new artifact digest for the profile's revision 2.
fn second_artifact() -> (String, Vec<u8>) {
    let manifest = "api_version: shaula.io/template-profile/v1\nkind: RunnerTemplateProfile\nplatform: kubernetes\nruntime:\n  protocol: terraform-cli/v1\n  engine: terraform\n  root_module: .\n  required_version: \">= 1.9, < 2.0\"\nbindings_contract: shaula.bindings.kubernetes/v1\ncontainer_bootstrap_contract: shaula.container-bootstrap/v1\nschemas:\n  bindings: schemas/bindings.schema.json\n  parameters: schemas/parameters.schema.json\nmanaged_resource_shape:\n  - role: bootstrap\n    terraform_type: kubernetes_secret_v1\n    exact_count: 1\n  - role: runner\n    terraform_type: kubernetes_pod_v1\n    exact_count: 1\nrunner_image_digests:\n  - ghcr.io/actions/actions-runner:2.323.0@sha256:3f2a1b9c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8\nruntime_policy_digest: sha256:policy-v2\n";
    let mut builder = tar::Builder::new(Vec::new());
    for (name, content) in [
        ("profile.yaml", manifest),
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
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    (format!("sha256:{}", hex::encode(hasher.finalize())), bytes)
}

/// Publishes and ACTIVATES the profile's revision 2 (auto-activation on
/// the scan cadence, spec 0017); returns the new artifact digest.
async fn activate_revision_2(
    app: &axum::Router,
    control_plane: &std::sync::Arc<shaula_store::registry_impl::SqliteControlPlane>,
) -> String {
    let (digest2, bytes2) = second_artifact();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/template-artifacts/{digest2}"))
        .header("authorization", common::oidc::bearer("template.publish"))
        .body(Body::from(bytes2))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    // A policy change keeps the revision distinct from a durable no-op.
    let body = template_put_body_r2(&digest2);
    assert_eq!(
        put_template_profile(app, "k8s-linux", body).await,
        StatusCode::ACCEPTED
    );
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

#[tokio::test]
async fn pool_put_resolves_members_to_current_active_revisions() {
    let (app, control_plane, _engine, _service) = build_app_with_service().await;
    let digest1 = seed_profile(&app, &control_plane, "k8s-linux", true).await;

    // Unknown member profile is a client error, never a 500.
    let bad = POOL_BODY.replace(r#""k8s-linux""#, r#""missing-profile""#);
    let response = app
        .clone()
        .oneshot(put_with_idempotency(
            "/api/v1/template-pools/builders",
            "pool-bad-1",
            bad,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let response = app
        .clone()
        .oneshot(put_with_idempotency(
            "/api/v1/template-pools/builders",
            "pool-1",
            POOL_BODY.into(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // The committed revision carries members resolved to the profiles'
    // CURRENT Active revisions with exact artifact pins (spec 0037 §3);
    // the stored spec keeps the bare keys.
    let revision = control_plane
        .template_pool_revision_latest("builders")
        .await
        .unwrap()
        .expect("pool revision committed");
    assert_eq!(revision.revision, 1);
    assert_eq!(revision.members.len(), 2);
    for member in &revision.members {
        assert_eq!(member.template_profile_key, "k8s-linux");
        assert_eq!(member.template_revision, 1);
        assert_eq!(member.template_artifact_digest, digest1);
    }
    assert!(revision.spec_json.contains(r#""k8s-linux""#));

    // A fleet referencing the pool freezes (pool_key, pool_revision) on
    // its committed revision (spec 0037 §4) and hydrates members from
    // the pool revision rows.
    let create = put_with_idempotency(
        "/api/v1/fleets/pool-fleet",
        "pool-fleet-1",
        fleet_body_with_pool_ref(),
    );
    let response = app.clone().oneshot(create).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let fleet_revision = control_plane
        .fleet_revision_latest("pool-fleet")
        .await
        .unwrap()
        .expect("fleet revision committed");
    assert_eq!(
        fleet_revision.template_pool_ref,
        Some(("builders".to_string(), 1))
    );
    assert_eq!(fleet_revision.template_pool.len(), 2);

    // An identical re-assertion with unchanged Active revisions is the
    // no-op shape: 200, no new pool revision.
    let etag = pool_etag(&app, "builders").await;
    let mut replace = authorized(
        "PUT",
        "/api/v1/template-pools/builders",
        Some(POOL_BODY.into()),
    );
    replace.headers_mut().remove("if-none-match");
    replace.headers_mut().insert(
        "if-match",
        axum::http::HeaderValue::from_str(&etag).unwrap(),
    );
    let response = app.clone().oneshot(replace).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let head = control_plane
        .template_pool_get("builders")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.desired_revision, 1);
}

#[tokio::test]
async fn pool_delete_is_blocked_while_a_live_fleet_references_it() {
    let (app, control_plane, _engine, _service) = build_app_with_service().await;
    seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let response = app
        .clone()
        .oneshot(put_with_idempotency(
            "/api/v1/template-pools/builders",
            "pool-1",
            POOL_BODY.into(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let create = put_with_idempotency(
        "/api/v1/fleets/pool-fleet",
        "pool-fleet-1",
        fleet_body_with_pool_ref(),
    );
    assert_eq!(
        app.clone().oneshot(create).await.unwrap().status(),
        StatusCode::ACCEPTED
    );

    // Referenced: the tombstone is refused with 409 ResourceInUse and
    // the pool stays readable (spec 0037 §7).
    let etag = pool_etag(&app, "builders").await;
    let mut delete = authorized("DELETE", "/api/v1/template-pools/builders", None);
    delete.headers_mut().insert(
        "if-match",
        axum::http::HeaderValue::from_str(&etag).unwrap(),
    );
    let response = app.clone().oneshot(delete).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let view = get_json(&app, "/api/v1/template-pools/builders").await;
    assert_eq!(view["metadata"]["revision"].as_i64().unwrap(), 1);

    // Unreferenced: the tombstone lands and later reads are 410 Gone.
    let fleet_etag = {
        let response = app
            .clone()
            .oneshot(authorized("GET", "/api/v1/fleets/pool-fleet", None))
            .await
            .unwrap();
        response
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .trim_matches('"')
            .to_string()
    };
    let mut decommission = authorized("DELETE", "/api/v1/fleets/pool-fleet", None);
    decommission.headers_mut().insert(
        "if-match",
        axum::http::HeaderValue::from_str(&fleet_etag).unwrap(),
    );
    assert_eq!(
        app.clone().oneshot(decommission).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    let etag = pool_etag(&app, "builders").await;
    let mut delete = authorized("DELETE", "/api/v1/template-pools/builders", None);
    delete.headers_mut().insert(
        "if-match",
        axum::http::HeaderValue::from_str(&etag).unwrap(),
    );
    let response = app.clone().oneshot(delete).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response = app
        .clone()
        .oneshot(authorized("GET", "/api/v1/template-pools/builders", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GONE);
}

#[tokio::test]
async fn cascade_re_resolves_the_pool_and_catches_fleets_up_when_drained() {
    let (app, control_plane, _engine, service) = build_app_with_service().await;
    seed_profile(&app, &control_plane, "k8s-linux", true).await;
    let response = app
        .clone()
        .oneshot(put_with_idempotency(
            "/api/v1/template-pools/builders",
            "pool-1",
            POOL_BODY.into(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let create = put_with_idempotency(
        "/api/v1/fleets/pool-fleet",
        "pool-fleet-1",
        fleet_body_with_pool_ref(),
    );
    assert_eq!(
        app.clone().oneshot(create).await.unwrap().status(),
        StatusCode::ACCEPTED
    );

    let digest2 = activate_revision_2(&app, &control_plane).await;

    // Stage 1: the member's new Active revision mints ONE new pool
    // revision with re-pinned members (spec 0037 §5); the idle fleet
    // catches up in the same pass, so the counter reports both.
    let upgraded = service
        .cascade_template_follow_upgrades(1_800_000_003_000)
        .await
        .unwrap();
    assert_eq!(upgraded, 2);
    let pool_head = control_plane
        .template_pool_get("builders")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pool_head.desired_revision, 2);
    let pool_revision = control_plane
        .template_pool_revision_latest("builders")
        .await
        .unwrap()
        .unwrap();
    for member in &pool_revision.members {
        assert_eq!(member.template_revision, 2);
        assert_eq!(member.template_artifact_digest, digest2);
    }

    // Stage 2: the idle referencing fleet catches up to pool revision 2
    // on the SAME pass (its occupancy is zero), freezing the new routing
    // context; hydrated members carry the new pins.
    let fleet_revision = control_plane
        .fleet_revision_latest("pool-fleet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fleet_revision.revision, 2);
    assert_eq!(
        fleet_revision.template_pool_ref,
        Some(("builders".to_string(), 2))
    );
    assert!(fleet_revision
        .template_pool
        .iter()
        .all(|m| m.template_artifact_digest == digest2));

    // No lag remains: a second pass mints nothing.
    let again = service
        .cascade_template_follow_upgrades(1_800_000_004_000)
        .await
        .unwrap();
    assert_eq!(again, 0);

    // An occupied fleet defers its catch-up: occupy, mint pool revision
    // 3 by re-resolving (simulate via a pool PUT — an explicit admission
    // resolving the same Active), and verify the fleet stays behind
    // until the drain.
    let now = 1_800_000_004_500i64;
    let revision = control_plane
        .fleet_revision_latest("pool-fleet")
        .await
        .unwrap()
        .unwrap();
    control_plane
        .generation_insert(shaula_core::registry::GenerationRecord {
            id: "gen-pool-occupied".into(),
            fleet_key: "pool-fleet".into(),
            runner_name: "runner-pool".into(),
            generation_name: "occupied".into(),
            fleet_revision: 2,
            pool_member_key: Some("primary".into()),
            template_profile_key: "k8s-linux".into(),
            template_revision: 2,
            template_artifact_digest: digest2.clone(),
            attestation_id: "static-validation-v1:test".into(),
            inputs_digest: revision.inputs_digest.clone(),
            state: shaula_core::lifecycle::GenerationState::Idle,
            github_runner_id: None,
            workspace_path: "/tmp/unused".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    // A weight edit mints pool revision 3 with the same member pins.
    let etag = pool_etag(&app, "builders").await;
    let heavier = POOL_BODY.replace(r#""weight": 10,"#, r#""weight": 30,"#);
    let mut replace = authorized("PUT", "/api/v1/template-pools/builders", Some(heavier));
    replace.headers_mut().remove("if-none-match");
    replace.headers_mut().insert(
        "if-match",
        axum::http::HeaderValue::from_str(&etag).unwrap(),
    );
    assert_eq!(
        app.clone().oneshot(replace).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    let deferred = service
        .cascade_template_follow_upgrades(1_800_000_005_000)
        .await
        .unwrap();
    assert_eq!(deferred, 0);
    let fleet_revision = control_plane
        .fleet_revision_latest("pool-fleet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fleet_revision.template_pool_ref,
        Some(("builders".to_string(), 2)),
        "occupied fleet must keep its frozen pool revision"
    );

    // Drain along the legal lifecycle path (insert always lands as
    // CreatePending at the persistence boundary).
    for state in [
        shaula_core::lifecycle::GenerationState::Creating,
        shaula_core::lifecycle::GenerationState::WaitingOnline,
        shaula_core::lifecycle::GenerationState::Idle,
        shaula_core::lifecycle::GenerationState::Retiring,
        shaula_core::lifecycle::GenerationState::DestroyPending,
        shaula_core::lifecycle::GenerationState::Destroying,
        shaula_core::lifecycle::GenerationState::Destroyed,
    ] {
        control_plane
            .generation_advance("gen-pool-occupied", state, now)
            .await
            .unwrap();
    }
    let caught_up = service
        .cascade_template_follow_upgrades(1_800_000_006_000)
        .await
        .unwrap();
    assert_eq!(caught_up, 1);
    let fleet_revision = control_plane
        .fleet_revision_latest("pool-fleet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fleet_revision.template_pool_ref,
        Some(("builders".to_string(), 3))
    );
}

/// Regression (spec 0037 §4/§5): a capacity-only PUT on a
/// `template_pool_ref` fleet must NOT re-freeze to the pool's latest
/// revision and demand zero occupancy. Catch-up is the cascade's
/// deferred job; a fleet update that changes only capacity is accepted
/// while jobs are running and retains the frozen pool routing pin.
#[tokio::test]
async fn capacity_only_put_keeps_frozen_pool_revision_under_occupancy() {
    let (app, control_plane, _engine, service) = build_app_with_service().await;
    seed_profile(&app, &control_plane, "k8s-linux", true).await;
    assert_eq!(
        app.clone()
            .oneshot(put_with_idempotency(
                "/api/v1/template-pools/builders",
                "pool-1",
                POOL_BODY.into(),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        app.clone()
            .oneshot(put_with_idempotency(
                "/api/v1/fleets/pool-fleet",
                "pool-fleet-1",
                fleet_body_with_pool_ref(),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );

    // Occupy the fleet so any "requires zero occupancy" gate would trip.
    let now = 1_800_000_010_000i64;
    let revision = control_plane
        .fleet_revision_latest("pool-fleet")
        .await
        .unwrap()
        .unwrap();
    control_plane
        .generation_insert(shaula_core::registry::GenerationRecord {
            id: "gen-cap".into(),
            fleet_key: "pool-fleet".into(),
            runner_name: "runner-cap".into(),
            generation_name: "cap".into(),
            fleet_revision: 1,
            pool_member_key: Some("primary".into()),
            template_profile_key: "k8s-linux".into(),
            template_revision: 1,
            template_artifact_digest: "sha256:cap".into(),
            attestation_id: "static-validation-v1:test".into(),
            inputs_digest: revision.inputs_digest.clone(),
            state: shaula_core::lifecycle::GenerationState::Idle,
            github_runner_id: None,
            workspace_path: "/tmp/unused".into(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    // Mint pool revision 2 (a weight edit re-resolves the same Active
    // pins). The fleet now lags behind the pool head.
    let etag = pool_etag(&app, "builders").await;
    let heavier = POOL_BODY.replace(r#""weight": 10,"#, r#""weight": 30,"#);
    let mut replace = authorized("PUT", "/api/v1/template-pools/builders", Some(heavier));
    replace.headers_mut().remove("if-none-match");
    replace.headers_mut().insert(
        "if-match",
        axum::http::HeaderValue::from_str(&etag).unwrap(),
    );
    assert_eq!(
        app.clone().oneshot(replace).await.unwrap().status(),
        StatusCode::ACCEPTED
    );

    // A capacity-only fleet PUT is accepted WITHOUT draining and must
    // keep the fleet's frozen pool revision 1 — it did not switch pools.
    let fleet_etag = {
        let response = app
            .clone()
            .oneshot(authorized("GET", "/api/v1/fleets/pool-fleet", None))
            .await
            .unwrap();
        response
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .trim_matches('"')
            .to_string()
    };
    let mut update = authorized(
        "PUT",
        "/api/v1/fleets/pool-fleet",
        Some(fleet_body_with_pool_ref_bigger()),
    );
    update.headers_mut().remove("if-none-match");
    update.headers_mut().insert(
        "if-match",
        axum::http::HeaderValue::from_str(&fleet_etag).unwrap(),
    );
    update.headers_mut().insert(
        "idempotency-key",
        axum::http::HeaderValue::from_str("pool-fleet-2").unwrap(),
    );
    let response = app.clone().oneshot(update).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "capacity-only PUT must not be drain-blocked: {}",
        String::from_utf8_lossy(&body)
    );
    let fleet_revision = control_plane
        .fleet_revision_latest("pool-fleet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fleet_revision.template_pool_ref,
        Some(("builders".to_string(), 1)),
        "capacity-only PUT retains the frozen pool revision"
    );
    // The cascade, not PUT, owns catch-up once the fleet drains.
    let _ = service;
}
