//! Exercise exact-pin materials through publication and follow, not helpers.
use super::*;

#[tokio::test]
async fn template_input_reads_precede_backend_but_validation_follows_it() {
    let (app, store, _engine, root) = build_app_with_artifact_root().await;
    let digest = seed_profile(&app, &store, "k8s-linux", true).await;
    let artifact = shaula_core::artifact_layout::artifact_dir(&root, &digest).unwrap();
    let schema = artifact.join("schemas/parameters.schema.json");
    let manifest = artifact.join("profile.yaml");
    let original = std::fs::read(&manifest).unwrap();
    std::fs::write(&manifest, "not a manifest").unwrap();
    std::fs::write(&schema, " \n").unwrap();
    // Backend validation rejects first, even though input material is blank.
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/fleets/linux-x64",
                Some(FLEET_BODY.into()),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // A read failure, unlike blank bytes, precedes the backend gate.
    std::fs::remove_file(&schema).unwrap();
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/fleets/linux-x64",
                Some(FLEET_BODY.into()),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    std::fs::write(&manifest, original).unwrap();
    std::fs::write(&schema, " \n").unwrap();
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/fleets/linux-x64",
                Some(FLEET_BODY.into()),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(store.fleet_get("linux-x64").await.unwrap().is_none());
    std::fs::write(&schema, "{}").unwrap();
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/fleets/linux-x64",
                Some(FLEET_BODY.into()),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
}

fn pool_body() -> String {
    serde_json::json!({
        "members": [{
            "key": "primary", "template_profile_ref": "k8s-linux",
            "weight": 1, "template_inputs": {"size_class": "standard"}
        }],
        "failure_policy": "backpressure"
    })
    .to_string()
}

#[tokio::test]
async fn template_input_material_failures_block_put_and_follow_until_restored() {
    let (app, store, engine, service) = build_app_with_service().await;
    seed_profile(&app, &store, "k8s-linux", true).await;
    assert_eq!(
        app.clone()
            .oneshot(put_with_idempotency(
                "/api/v1/fleets/linux-x64",
                "materials-create",
                FLEET_BODY.into(),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/template-pools/builders",
                Some(pool_body()),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let digest = publish_revision_2(&app, &store, None).await;
    let root = engine.parent().unwrap().parent().unwrap().join("artifacts");
    let schema = shaula_core::artifact_layout::artifact_dir(&root, &digest)
        .unwrap()
        .join("schemas/parameters.schema.json");
    let original = std::fs::read(&schema).unwrap();
    // Blank bytes and a read failure must remain storage failures, not an
    // unconstrained schema or client rejection. This fixture has no DB cache
    // restorer, so the on-disk fault reaches the real SQLite adapter.
    for missing in [false, true] {
        if missing {
            std::fs::remove_file(&schema).unwrap();
        } else {
            std::fs::write(&schema, " \n\t").unwrap();
        }
        let changed = FLEET_BODY.replace(
            r#""template_inputs": {}"#,
            r#""template_inputs": {"size_class": "standard"}"#,
        );
        let request = authorized_fleet_put(&app, "/api/v1/fleets/linux-x64", changed).await;
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            app.clone()
                .oneshot(authorized(
                    "PUT",
                    "/api/v1/template-pools/rejected",
                    Some(pool_body()),
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            service
                .cascade_template_follow_upgrades(1_800_000_003_000)
                .await
                .unwrap(),
            0
        );
        assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (1, 1));
        assert_eq!(
            store
                .template_pool_get("builders")
                .await
                .unwrap()
                .unwrap()
                .desired_revision,
            1
        );
        assert!(store.template_pool_get("rejected").await.unwrap().is_none());
        std::fs::write(&schema, &original).unwrap();
    }
    // The exact same artifact, now readable, unblocks both follow paths.
    assert_eq!(
        service
            .cascade_template_follow_upgrades(1_800_000_004_000)
            .await
            .unwrap(),
        2
    );
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (2, 2));
    assert_eq!(
        store
            .template_pool_get("builders")
            .await
            .unwrap()
            .unwrap()
            .desired_revision,
        2
    );
    assert_eq!(
        service
            .cascade_template_follow_upgrades(1_800_000_005_000)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn template_input_policy_rejection_is_consistent_across_put_and_follow() {
    let (app, store, _engine, service) = build_app_with_service().await;
    seed_profile(&app, &store, "k8s-linux", true).await;
    let body = FLEET_BODY.replace(
        r#""template_inputs": {}"#,
        r#""template_inputs": {"size_class": "standard"}"#,
    );
    assert_eq!(
        app.clone()
            .oneshot(put_with_idempotency(
                "/api/v1/fleets/linux-x64",
                "policy-create",
                body.clone(),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/template-pools/builders",
                Some(pool_body()),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    publish_revision_2(&app, &store, Some(r#"{"size_class":["enterprise"]}"#)).await;
    // New callers resolve Active and reject the old value. Existing followers
    // retain their exact pin and defer rather than publishing partial changes.
    assert_eq!(
        app.clone()
            .oneshot(put_with_idempotency(
                "/api/v1/fleets/new-fleet",
                "policy-rejected",
                body,
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/template-pools/rejected",
                Some(pool_body()),
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        service
            .cascade_template_follow_upgrades(1_800_000_003_000)
            .await
            .unwrap(),
        0
    );
    assert_eq!(fleet_revision_pin(&app, "linux-x64").await, (1, 1));
    assert_eq!(
        store
            .template_pool_get("builders")
            .await
            .unwrap()
            .unwrap()
            .desired_revision,
        1
    );
    assert!(store.fleet_get("new-fleet").await.unwrap().is_none());
    assert!(store.template_pool_get("rejected").await.unwrap().is_none());
}
