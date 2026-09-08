//! Real router/store/projection/admission coverage for spec 0014.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "support/template_library_reads.rs"]
mod library_reads;
#[path = "support/input_contract.rs"]
mod support;

use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode};
use serde_json::json;
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

use common::*;
use support::{publish, response};

#[tokio::test]
async fn exact_revision_reads_are_private_authorized_and_do_not_follow_the_head() {
    let (app, store, _) = build_app_with_scan().await;
    let schema = r#"{"type":"object","required":["runner_image"],"additionalProperties":false,
        "properties":{"runner_image":{"title":"Runner image","type":"string","enum":["a","b","c"]},
        "cpu":{"type":"integer"}}}"#;
    let digest1 = publish(
        &app,
        "visual",
        schema,
        json!({"runner_image":["b","a","b"],"cpu":[9007199254740993_u64]}),
    )
    .await;
    let uri = "/api/v1/template-profiles/visual/revisions/1/input-contract";
    let (status, first) = response(&app, uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["version"], 1);
    assert_eq!(first["profileKey"], "visual");
    assert_eq!(first["revision"], 1);
    assert_eq!(first["artifactDigest"], digest1);
    assert_eq!(first["mode"], "fields");
    assert_eq!(
        first["fields"][0]["options"][0]["valueJson"],
        "9007199254740993"
    );
    assert_eq!(
        first["fields"][1]["options"],
        json!([{"valueJson":"\"b\""},{"valueJson":"\"a\""}])
    );
    assert_eq!(first["fields"][1]["required"], true);
    assert_eq!(
        first["incarnation"],
        store
            .template_profile_get("visual")
            .await
            .unwrap()
            .unwrap()
            .incarnation
    );
    let digest2 = publish(&app, "visual", "{}", json!({"new":[false]})).await;
    assert_ne!(digest1, digest2);
    assert_eq!(
        response(&app, uri).await.1,
        first,
        "old revision keeps its original materials"
    );
    let second = response(
        &app,
        "/api/v1/template-profiles/visual/revisions/2/input-contract",
    )
    .await
    .1;
    assert_eq!(second["artifactDigest"], digest2);
    assert_eq!(second["fields"][0]["key"], "new");
    let profile = store.template_profile_get("visual").await.unwrap().unwrap();
    assert_eq!(profile.desired_revision, 2);
    assert_eq!(
        profile.active_revision, None,
        "reads cannot create validation or activation"
    );

    let unauthenticated = Request::builder().uri(uri).body(Body::empty()).unwrap();
    assert_eq!(
        app.clone().oneshot(unauthenticated).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let mut forbidden = authorized("GET", uri, None);
    forbidden.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&oidc::bearer("fleet.write")).unwrap(),
    );
    assert_eq!(
        app.clone().oneshot(forbidden).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    for uri in [
        "/api/v1/template-profiles/missing/revisions/1/input-contract",
        "/api/v1/template-profiles/visual/revisions/99/input-contract",
    ] {
        assert_eq!(response(&app, uri).await.0, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn unavailable_contracts_and_storage_failures_never_disguise_themselves_as_empty_inputs() {
    let (app, _store, _engine, root) = build_app_with_artifact_root().await;
    let digest = publish(&app, "empty", "{}", json!({})).await;
    let uri = "/api/v1/template-profiles/empty/revisions/1/input-contract";
    let (status, view) = response(&app, uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["fields"], json!([]));
    let schema_file = shaula_core::artifact_layout::artifact_dir(&root, &digest)
        .unwrap()
        .join("schemas/parameters.schema.json");
    for schema in [
        r#"{"$ref":"SECRET_SCHEMA_VALUE"}"#,
        r#"{"required":["unavailable"]}"#,
        "false",
        "{broken",
    ] {
        std::fs::write(&schema_file, schema).unwrap();
        let (status, error) = response(&app, uri).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "InputContractUnavailable");
        assert!(!error.to_string().contains("SECRET_SCHEMA_VALUE"));
        let metadata = get_json(&app, "/api/v1/template-profiles/empty/revisions/1").await;
        assert_eq!(
            metadata["artifactDigest"], digest,
            "metadata survives projection failure"
        );
    }
    std::fs::write(&schema_file, "  \n\t").unwrap();
    assert_eq!(
        response(&app, uri).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    std::fs::remove_file(&schema_file).unwrap();
    assert_eq!(
        response(&app, uri).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    std::fs::create_dir(&schema_file).unwrap();
    assert_eq!(
        response(&app, uri).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    std::fs::remove_dir(&schema_file).unwrap();
    std::fs::write(&schema_file, "{}").unwrap();
    assert_eq!(response(&app, uri).await.0, StatusCode::OK);

    publish(&app, "malformed-policy", "{}", json!({"unused":false})).await;
    assert_eq!(
        response(
            &app,
            "/api/v1/template-profiles/malformed-policy/revisions/1/input-contract"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn complete_preset_selection_still_passes_through_real_fleet_admission() {
    let (app, store, engine) = build_app_with_scan().await;
    let schema = r#"{"type":"object","required":["image","config"],"enum":[
        {"image":"a","config":{"cpu":1}},{"image":"b","config":{"cpu":2}},
        {"image":"unapproved","config":{"cpu":1}}],
        "properties":{"config":{"type":"object","required":["cpu"],"additionalProperties":false,
        "properties":{"cpu":{"type":"integer"}}}},"additionalProperties":true}"#;
    let digest = publish(
        &app,
        "k8s-linux",
        schema,
        json!({"image":["a","b"],"config":[{"cpu":1},{"cpu":2}]}),
    )
    .await;
    let contract = response(
        &app,
        "/api/v1/template-profiles/k8s-linux/revisions/1/input-contract",
    )
    .await
    .1;
    assert_eq!(contract["mode"], "presets");
    assert!(contract.get("fields").is_none());
    assert_eq!(contract["presets"].as_array().unwrap().len(), 2);
    assert_eq!(
        app.clone()
            .oneshot(authorized(
                "PUT",
                "/api/v1/github-auth-profiles/prod-app",
                Some(AUTH_PUT_BODY.into())
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    store.periodic_scan(1_800_000_001_000).await.unwrap();
    store
        .auth_apply_validation("prod-app", 1, true, None, 1_800_000_001_500)
        .await
        .unwrap();
    let attestation = attest_body(
        &store,
        &digest,
        &expected_bindings_digest("k8s-linux", 1),
        &engine,
    )
    .await;
    assert_eq!(
        app.clone()
            .oneshot(attestation_put_request(
                "/api/v1/template-profiles/k8s-linux/revisions/1/attestations/input-contract",
                attestation
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        get_json(&app, "/api/v1/template-profiles/k8s-linux").await["activeRevision"],
        1
    );
    for (name, inputs, expected) in [
        (
            "valid",
            json!({"image":"a","config":{"cpu":1}}),
            StatusCode::ACCEPTED,
        ),
        (
            "cross-product",
            json!({"image":"a","config":{"cpu":2}}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "nested",
            json!({"image":"a","config":{}}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("missing", json!({}), StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let mut body: serde_json::Value = serde_json::from_str(FLEET_BODY).unwrap();
        body["template_profile_ref"] = json!({"key":"k8s-linux","revision":1});
        body["template_inputs"] = inputs;
        body["github"]["scale_set_name"] = json!(name);
        let response = app
            .clone()
            .oneshot(authorized(
                "PUT",
                &format!("/api/v1/fleets/{name}"),
                Some(body.to_string()),
            ))
            .await
            .unwrap();
        let status = response.status();
        let detail = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(
            status,
            expected,
            "{name}: {}",
            String::from_utf8_lossy(&detail)
        );
    }
    assert!(store.fleet_get("cross-product").await.unwrap().is_none());
}
