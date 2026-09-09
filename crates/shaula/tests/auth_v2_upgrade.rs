//! Unsupported authentication formats remain history and cannot authorize work.

#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
#[path = "support/unsupported_auth.rs"]
mod history;

use axum::http::StatusCode;
use common::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const LEGACY: &str = r#"{"kind":"github_app","app_id":"4863460","installation_id":34,"private_key":"historical-inert-credential","target_allowlist":[{"kind":"organization","owner":"example-org"}]}"#;
const URI: &str = "/api/v1/github-auth-profiles/old-app";

#[tokio::test]
async fn old_http_requests_cannot_publish_or_replay() -> TestResult {
    let (app, store, engine) = build_app_with_scan().await;
    history::seed(&engine, "old-app", "github_app").await?;
    let db = history::database(&engine).await?;
    let canonical =
        r#"github_app|4863460|34||{"targets":[{"kind":"organization","owner":"example-org"}]}"#;
    let hash = shaula_core::auth::request_hash_parts(&[
        b"github_auth_profile",
        b"old-app",
        b"legacy-retry",
        canonical.as_bytes(),
    ]);
    db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
        "INSERT INTO idempotency_records(resource_kind,resource_key,idempotency_key,request_hash,response_status,response_body,created_at) VALUES ('github_auth_profile','old-app','legacy-retry',?,202,?,1)",
        [hash.into(), r#"{"etag":"old-app-inc:1","change":{"id":"old-change","resource_kind":"github_auth_profile","resource_key":"old-app","revision":1,"kind":"Rotate","state":"Pending","reason":null},"no_op":false}"#.into()])).await?;
    db.close().await?;
    for body in [
        LEGACY.to_string(),
        LEGACY.replace("\"installation_id\":34", "\"installation_id\":null"),
        r#"{"kind":"pat","token":"fixture","pat_principal":"old","target_allowlist":[]}"#.into(),
    ] {
        let response = app
            .clone()
            .oneshot(put_with_idempotency(URI, "legacy-retry", body))
            .await?;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert_eq!(
        store
            .auth_profile_get("old-app")
            .await?
            .unwrap()
            .desired_revision,
        1
    );
    Ok(())
}

#[tokio::test]
async fn old_active_revisions_are_visible_but_cannot_be_referenced_or_upgraded() -> TestResult {
    let (app, store, engine) = build_app_with_scan().await;
    common::attestation_harness::seed_profile(&app, &store, "k8s-linux", true).await;
    for (key, kind) in [("old-app", "github_app"), ("old-pat", "pat")] {
        history::seed(&engine, key, kind).await?;
        let uri = format!("/api/v1/github-auth-profiles/{key}");
        let view = get_json(&app, &uri).await;
        assert_eq!(view["status"], "Unsupported");
        assert_eq!(view["active"]["state"], "Unsupported");
        assert!(view["active"].get("target_policy").is_none());
        for field in ["identity", "target_allowlist"] {
            assert!(view.get(field).is_none());
            assert!(view["active"].get(field).is_none());
        }
        let revision = get_json(&app, &format!("{uri}/revisions/1")).await;
        assert_eq!(revision["state"], "Unsupported");
        assert!(revision.get("target_policy").is_none());
        assert!(revision.get("installationId").is_none());
        assert!(revision.get("patPrincipal").is_none());
        let mut fleet: serde_json::Value = serde_json::from_str(FLEET_BODY)?;
        fleet["github"]["auth_profile_ref"] = serde_json::json!(key);
        let response = app
            .clone()
            .oneshot(authorized(
                "PUT",
                &format!("/api/v1/fleets/{key}"),
                Some(fleet.to_string()),
            ))
            .await?;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let get = app.clone().oneshot(authorized("GET", &uri, None)).await?;
        let mut publish = authorized("PUT", &uri, Some(AUTH_PUT_BODY.into()));
        publish.headers_mut().remove("if-none-match");
        publish
            .headers_mut()
            .insert("if-match", get.headers()["etag"].clone());
        assert_eq!(
            app.clone().oneshot(publish).await?.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            store.auth_profile_get(key).await?.unwrap().desired_revision,
            1
        );
    }
    let list = get_json(&app, "/api/v1/github-auth-profiles").await;
    for key in ["old-app", "old-pat"] {
        let profile = list["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["key"] == key)
            .unwrap();
        assert_eq!(profile["status"], "Unsupported");
    }
    Ok(())
}
