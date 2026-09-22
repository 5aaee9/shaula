//! Publisher backend selection on the existing container source, through HTTP + SQLite.
#[expect(
    clippy::unwrap_used,
    reason = "existing shared assertion-style HTTP fixtures"
)]
mod common;

use axum::http::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use shaula_core::registry::{AuthPromotion, AuthPromotionOutcome, ControlPlaneStore};
use tower::ServiceExt;

use common::attestation_harness::{put_template_profile, seed_profile_artifact};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn artifact() -> TestResult<(String, Vec<u8>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/kubernetes");
    let mut builder = tar::Builder::new(Vec::new());
    for name in [
        "profile.yaml",
        "main.tf",
        ".terraform.lock.hcl",
        "runtime-policy.md",
        "schemas/bindings.schema.json",
        "schemas/parameters.schema.json",
    ] {
        builder.append_path_with_name(root.join(name), name)?;
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &builder.into_inner()?)?;
    let bytes = encoder.finish()?;
    Ok((
        format!("sha256:{}", hex::encode(Sha256::digest(&bytes))),
        bytes,
    ))
}

fn publication(digest: &str, backend: &str) -> String {
    json!({"artifact_digest":digest,"engine_ref":"terraform",
        "bindings":{"namespace":"runners","kubeconfig":"protected-path","runner_backend":backend},
        "fleet_input_policy":{}})
    .to_string()
}

async fn put_fleet(app: &axum::Router, key: &str, mut body: Value) -> TestResult<StatusCode> {
    if let Some(github) = body.get_mut("github") {
        github["scale_set_name"] = json!(key);
    }
    Ok(app
        .clone()
        .oneshot(common::authorized(
            "PUT",
            &format!("/api/v1/fleets/{key}"),
            Some(body.to_string()),
        ))
        .await?
        .status())
}

fn github(profile: &str) -> TestResult<Value> {
    let mut body: Value = serde_json::from_str(common::FLEET_BODY)?;
    body["template_profile_ref"] = json!(profile);
    Ok(body)
}

fn forgejo(profile: &str) -> Value {
    json!({"kind":"forgejo","forgejo":{"instance_url":"https://forgejo.test",
        "scope":{"kind":"instance"},"auth_profile_ref":"forgejo-auth",
        "runner_name_prefix":"forgejo-", "labels":["linux:host"]},
        "capacity":{"min_runners":0,"max_runners":2},"template_profile_ref":profile})
}

#[tokio::test]
async fn one_source_publishes_both_backends_and_rejects_mismatched_fleets_and_images() -> TestResult
{
    let (app, store, _engine, _service) = common::build_app_with_service().await;
    let (digest, bytes) = artifact()?;
    seed_profile_artifact(&app, &store, "k8s-linux", digest.clone(), bytes, true).await;
    assert_eq!(
        put_template_profile(&app, "forgejo-linux", publication(&digest, "forgejo")).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        put_template_profile(&app, "invalid", publication(&digest, "other")).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    store.periodic_scan(1_800_000_002_000).await?;
    for key in ["k8s-linux", "forgejo-linux"] {
        assert_eq!(
            store
                .template_profile_get(key)
                .await?
                .ok_or("profile missing")?
                .active_revision,
            Some(1)
        );
        assert_eq!(
            store
                .template_revision_get(key, 1)
                .await?
                .ok_or("revision missing")?
                .artifact_digest,
            digest
        );
    }
    let response = app
        .clone()
        .oneshot(common::authorized("GET", "/api/v1/template-profiles", None))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body: Value = serde_json::from_slice(&body)?;
    let profiles = body["profiles"].as_array().ok_or("profiles missing")?;
    for (key, backend) in [("k8s-linux", "github"), ("forgejo-linux", "forgejo")] {
        let profile = profiles
            .iter()
            .find(|p| p["key"] == key)
            .ok_or("profile missing")?;
        assert_eq!(profile["runnerBackend"], backend);
        assert!(profile.get("bindings").is_none());
    }
    let (bindings, _) = store
        .template_protected_bindings("forgejo-linux", 1)
        .await?
        .ok_or("bindings missing")?;
    assert_eq!(
        serde_json::from_str::<Value>(&bindings)?["runner_backend"],
        "forgejo"
    );
    assert_eq!(
        put_fleet(&app, "github-default", github("k8s-linux")?).await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        put_fleet(&app, "github-wrong", github("forgejo-linux")?).await?,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        put_fleet(&app, "forgejo-wrong", forgejo("k8s-linux")).await?,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let auth = json!({"kind":"forgejo_token","instance_url":"https://forgejo.test",
        "scope":{"kind":"instance"},"token":"protected-test-token"});
    let response = app
        .clone()
        .oneshot(common::authorized(
            "PUT",
            "/api/v1/github-auth-profiles/forgejo-auth",
            Some(auth.to_string()),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    // The periodic structural scan must not race the provider's online probe
    // and reject a valid Forgejo credential as an unsupported GitHub schema.
    let scan = store.periodic_scan(1_800_000_002_000).await?;
    assert_eq!(scan.auth_rejected, 0);
    let pending = store
        .auth_profile_get("forgejo-auth")
        .await?
        .ok_or("auth missing")?;
    assert_eq!(pending.status, "Validating");
    assert_eq!(pending.active_revision, None);
    let probe = shaula_core::ports::forgejo::ForgejoAuthProbe {
        server_version: "16.0.4".into(),
        principal_id: None,
        target_id: None,
        checked_at_unix_ms: 1_800_000_002_000,
        valid_until_unix_ms: 1_800_000_062_000,
        runner_count: 0,
    };
    assert_eq!(
        store
            .auth_apply_validation_v2(
                "forgejo-auth",
                1,
                true,
                None,
                1_800_000_002_000,
                Some(AuthPromotion {
                    bindings: vec![],
                    snapshot_json: serde_json::to_string(&probe)?
                })
            )
            .await?,
        AuthPromotionOutcome::Promoted
    );
    assert_eq!(
        put_fleet(&app, "forgejo-good", forgejo("forgejo-linux")).await?,
        StatusCode::ACCEPTED
    );
    let mut wrong_image = forgejo("forgejo-linux");
    wrong_image["forgejo"]["runner_name_prefix"] = json!("wrong-image-");
    wrong_image["template_inputs"] =
        json!({"runner_image":"ghcr.io/actions/actions-runner:2.337.0"});
    assert_eq!(
        put_fleet(&app, "forgejo-image", wrong_image).await?,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut override_backend = github("k8s-linux")?;
    override_backend["template_inputs"] = json!({"runner_backend":"forgejo"});
    assert_eq!(
        put_fleet(&app, "backend-override", override_backend).await?,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    Ok(())
}

#[tokio::test]
async fn forgejo_pool_requests_fail_before_resolving_or_persisting_dependencies() -> TestResult {
    let (app, store, _engine, _service) = common::build_app_with_service().await;
    for (field, value) in [
        ("template_pool_ref", json!("nonexistent")),
        (
            "template_pool",
            json!({"members":[{"key":"a","template_profile_ref":"nonexistent","weight":1,"template_inputs":{}}],"failure_policy":"backpressure"}),
        ),
    ] {
        let mut body = forgejo("");
        body.as_object_mut()
            .ok_or("object expected")?
            .remove("template_profile_ref");
        body[field] = value;
        let response = app
            .clone()
            .oneshot(common::authorized(
                "PUT",
                "/api/v1/fleets/unsupported-pool",
                Some(body.to_string()),
            ))
            .await?;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
        let error: Value = serde_json::from_slice(&body)?;
        assert!(error["detail"]
            .as_str()
            .ok_or("detail missing")?
            .contains("template pools are unsupported"));
        assert!(store.fleet_get("unsupported-pool").await?.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn follow_never_switches_a_single_or_shared_pool_fleet_to_the_other_backend() -> TestResult {
    let (app, store, _engine, service) = common::build_app_with_service().await;
    let (digest, bytes) = artifact()?;
    seed_profile_artifact(&app, &store, "k8s-linux", digest.clone(), bytes, true).await;
    assert_eq!(
        put_fleet(&app, "single", github("k8s-linux")?).await?,
        StatusCode::ACCEPTED
    );
    let pool = json!({"members":[{"key":"main","template_profile_ref":"k8s-linux","weight":1,"template_inputs":{}}],"failure_policy":"backpressure"});
    let response = app
        .clone()
        .oneshot(common::authorized(
            "PUT",
            "/api/v1/template-pools/builders",
            Some(pool.to_string()),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let mut pooled = github("k8s-linux")?;
    pooled
        .as_object_mut()
        .ok_or("fleet object")?
        .remove("template_profile_ref");
    pooled["template_pool_ref"] = json!("builders");
    assert_eq!(
        put_fleet(&app, "pooled", pooled.clone()).await?,
        StatusCode::ACCEPTED
    );

    // Same source/artifact and platform; only immutable publisher bindings move.
    assert_eq!(
        put_template_profile(&app, "k8s-linux", publication(&digest, "forgejo")).await,
        StatusCode::ACCEPTED
    );
    store.periodic_scan(1_800_000_002_000).await?;
    service
        .cascade_template_follow_upgrades(1_800_000_003_000)
        .await?;
    let single = store
        .fleet_revision_latest("single")
        .await?
        .ok_or("fleet missing")?;
    assert_eq!((single.revision, single.template_revision), (1, Some(1)));
    let pooled_revision = store
        .fleet_revision_latest("pooled")
        .await?
        .ok_or("pooled missing")?;
    assert_eq!(pooled_revision.revision, 1);
    assert_eq!(
        pooled_revision.template_pool_ref,
        Some(("builders".into(), 1))
    );
    assert_eq!(
        put_fleet(&app, "new-pooled", pooled).await?,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (old_bindings, _) = store
        .template_protected_bindings("k8s-linux", 1)
        .await?
        .ok_or("old bindings missing")?;
    assert!(serde_json::from_str::<Value>(&old_bindings)?
        .get("runner_backend")
        .is_none());
    Ok(())
}
