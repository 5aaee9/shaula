use super::{artifact, common, forgejo, publication, put_fleet, TestResult};
use axum::http::StatusCode;
use common::attestation_harness::{put_template_profile, seed_profile_artifact};
use serde_json::{json, Value};
use shaula_core::registry::{AuthPromotion, AuthPromotionOutcome, ControlPlaneStore};
use std::sync::Arc;
use tower::ServiceExt;

async fn fixture() -> TestResult<(
    axum::Router,
    Arc<shaula_store::registry_impl::SqliteControlPlane>,
    Arc<shaula_daemon::service::ControlPlane>,
    String,
)> {
    let (app, store, _, service) = common::build_app_with_service().await;
    let (digest, bytes) = artifact()?;
    seed_profile_artifact(&app, &store, "k8s-linux", digest.clone(), bytes, true).await;
    assert_eq!(
        put_template_profile(&app, "forgejo-linux", publication(&digest, "forgejo")).await,
        StatusCode::ACCEPTED
    );
    store.periodic_scan(1_800_000_002_000).await?;
    assert_eq!(app.clone().oneshot(common::authorized("PUT", "/api/v1/github-auth-profiles/forgejo-auth", Some(
        json!({"kind":"forgejo_token","instance_url":"https://forgejo.test","scope":{"kind":"instance"},"token":"test-token"}).to_string()
    ))).await?.status(), StatusCode::ACCEPTED);
    assert_eq!(store.auth_apply_validation_v2("forgejo-auth",1,true,None,1_800_000_002_000,Some(AuthPromotion {
        bindings: vec![], snapshot_json: json!({"server_version":"16.0.4","principal_id":null,"target_id":null,
            "checked_at_unix_ms":1_800_000_002_000_i64,"valid_until_unix_ms":1_800_000_062_000_i64,"runner_count":0}).to_string(),
    })).await?, AuthPromotionOutcome::Promoted);
    Ok((app, store, service, digest))
}

fn pool(profile: &str) -> Value {
    json!({"failure_policy":"redistribute","members":[
        {"key":"primary","template_profile_ref":profile,"weight":3,"max_runners":2,"template_inputs":{}},
        {"key":"secondary","template_profile_ref":profile,"weight":1,"max_runners":1,"template_inputs":{}}
    ]})
}

async fn put_pool(app: &axum::Router, key: &str, body: Value) -> TestResult {
    assert_eq!(
        app.clone()
            .oneshot(common::authorized(
                "PUT",
                &format!("/api/v1/template-pools/{key}"),
                Some(body.to_string())
            ))
            .await?
            .status(),
        StatusCode::ACCEPTED
    );
    Ok(())
}

fn fleet(key: &str, shared: bool, value: Value) -> Value {
    let mut spec = forgejo("");
    spec["forgejo"]["runner_name_prefix"] = json!(format!("{key}-"));
    spec[if shared {
        "template_pool_ref"
    } else {
        "template_pool"
    }] = value;
    spec
}

#[tokio::test]
async fn forgejo_admits_shared_and_inline_pools_but_rejects_mixed_backends_and_targets(
) -> TestResult {
    let (app, store, _, _) = fixture().await?;
    put_pool(&app, "forgejo-pool", pool("forgejo-linux")).await?;
    let mut mixed = pool("forgejo-linux");
    mixed["members"][1]["template_profile_ref"] = json!("k8s-linux");
    put_pool(&app, "mixed-pool", mixed.clone()).await?;
    for shared in [false, true] {
        let key = if shared { "shared" } else { "inline" };
        let spec = fleet(
            key,
            shared,
            if shared {
                json!("forgejo-pool")
            } else {
                pool("forgejo-linux")
            },
        );
        assert_eq!(put_fleet(&app, key, spec).await?, StatusCode::ACCEPTED);
        let frozen = store
            .fleet_revision_latest(key)
            .await?
            .ok_or("missing fleet revision")?;
        assert_eq!(frozen.template_profile_key, None);
        if shared {
            assert_eq!(frozen.template_pool_ref, Some(("forgejo-pool".into(), 1)));
        } else {
            assert_eq!(frozen.template_pool.len(), 2);
            assert!(frozen
                .template_pool
                .iter()
                .all(|member| member.template_profile_key == "forgejo-linux"));
        }
        let bad_key = format!("{key}-mixed");
        let bad = fleet(
            &bad_key,
            shared,
            if shared {
                json!("mixed-pool")
            } else {
                mixed.clone()
            },
        );
        assert_eq!(
            put_fleet(&app, &bad_key, bad).await?,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert!(store.fleet_get(&bad_key).await?.is_none());
        let bad_key = format!("{key}-targets");
        let mut bad = fleet(
            &bad_key,
            shared,
            if shared {
                json!("forgejo-pool")
            } else {
                pool("forgejo-linux")
            },
        );
        bad["forgejo"]["labels"] = json!(["linux:docker://node:22"]);
        assert_eq!(
            put_fleet(&app, &bad_key, bad).await?,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert!(store.fleet_get(&bad_key).await?.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn forgejo_pool_follow_retains_old_backend_pins_and_capacity_only_puts() -> TestResult {
    for shared in [true, false] {
        let (app, store, service, digest) = fixture().await?;
        put_pool(&app, "builders", pool("forgejo-linux")).await?;
        let mut spec = fleet(
            "shared",
            shared,
            if shared {
                json!("builders")
            } else {
                pool("forgejo-linux")
            },
        );
        assert_eq!(
            put_fleet(&app, "shared", spec.clone()).await?,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            put_template_profile(&app, "forgejo-linux", publication(&digest, "github")).await,
            StatusCode::ACCEPTED
        );
        store.periodic_scan(1_800_000_003_000).await?;
        service
            .cascade_template_follow_upgrades(1_800_000_004_000)
            .await?;
        assert_eq!(
            store
                .template_pool_revision_latest("builders")
                .await?
                .ok_or("missing pool")?
                .revision,
            2
        );
        let revision = store
            .fleet_revision_latest("shared")
            .await?
            .ok_or("missing fleet")?;
        assert_eq!(revision.revision, 1);
        assert_eq!(
            revision.template_pool_ref,
            shared.then(|| ("builders".into(), 1))
        );
        let view = app
            .clone()
            .oneshot(common::authorized("GET", "/api/v1/fleets/shared", None))
            .await?;
        let etag = view.headers().get("etag").ok_or("missing ETag")?.clone();
        spec["capacity"]["max_runners"] = json!(3);
        let mut put = common::authorized("PUT", "/api/v1/fleets/shared", Some(spec.to_string()));
        put.headers_mut().remove("if-none-match");
        put.headers_mut().insert("if-match", etag);
        assert_eq!(app.oneshot(put).await?.status(), StatusCode::ACCEPTED);
        assert_eq!(
            store
                .fleet_revision_latest("shared")
                .await?
                .ok_or("missing fleet")?
                .template_pool_ref,
            shared.then(|| ("builders".into(), 1))
        );
    }
    Ok(())
}
