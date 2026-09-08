//! Collection reads must expose stored profiles without hiding unreadable rows.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};
use serde_json::{json, Value};
use shaula_core::registry::ControlPlaneStore;
use tower::ServiceExt;

const COLLECTION: &str = "/api/v1/template-profiles";
type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn database_for(
    root: &std::path::Path,
) -> Result<DatabaseConnection, Box<dyn std::error::Error>> {
    let path = root
        .parent()
        .ok_or("artifact parent missing")?
        .join("data/test.db");
    Ok(Database::connect(format!(
        "sqlite://{}?mode=rw",
        path.to_string_lossy().replace('\\', "/")
    ))
    .await?)
}

async fn seed_profile(database: &DatabaseConnection, key: &str, status: &str) -> TestResult {
    // Metadata-only rows in this isolated read fixture need no artifact or worker.
    database.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO template_profiles (key,incarnation,desired_revision,active_revision,observed_revision,status,deletion_requested,created_at,updated_at) VALUES (?, ?, 1, 1, 1, ?, 0, 1, 1)",
        [key.into(), format!("incarnation-{key}").into(), status.into()],
    )).await?;
    database.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO template_profile_revisions (profile_key,revision,artifact_digest,engine_ref,state,created_at) VALUES (?,1,'fixture-artifact','terraform','Active',1)",
        [key.into()],
    )).await?;
    Ok(())
}

#[tokio::test]
async fn template_collection_orders_current_heads_without_filtering_existing_active_revisions(
) -> TestResult {
    let (app, _, _, root) = common::build_app_with_artifact_root().await;
    let database = database_for(&root).await?;
    for (key, status) in [("z-retiring", "Retiring"), ("a-upgrading", "Validating")] {
        seed_profile(&database, key, status).await?;
    }
    let response = app
        .oneshot(common::authorized("GET", COLLECTION, None))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    let body: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        body,
        json!({"profiles": [
            {"key": "a-upgrading", "incarnation": "incarnation-a-upgrading", "desiredRevision": 1,
                "activeRevision": 1, "status": "Validating"},
            {"key": "z-retiring", "incarnation": "incarnation-z-retiring", "desiredRevision": 1,
                "activeRevision": 1, "status": "Retiring"}
        ]})
    );
    database.close().await?;
    Ok(())
}

#[tokio::test]
async fn template_collection_propagates_revision_decode_fault_after_successful_key_enumeration(
) -> TestResult {
    let (app, store, _, root) = common::build_app_with_artifact_root().await;
    let database = database_for(&root).await?;
    for key in ["healthy-profile", "broken-profile"] {
        seed_profile(&database, key, "Active").await?;
    }
    // SQLite permits malformed UTF-8 TEXT. A damaged revision must fail the
    // real Store's row decoder while its profile head remains enumerable.
    database.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "UPDATE template_profile_revisions SET engine_ref=CAST(X'80' AS TEXT) WHERE profile_key='broken-profile'",
    )).await?;
    assert_eq!(store.template_profile_keys().await?.len(), 2);
    for (path, expected) in [
        (format!("{COLLECTION}/healthy-profile"), StatusCode::OK),
        (
            format!("{COLLECTION}/broken-profile"),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (COLLECTION.into(), StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let response = app
            .clone()
            .oneshot(common::authorized("GET", &path, None))
            .await?;
        assert_eq!(response.status(), expected, "{path}");
        if expected == StatusCode::INTERNAL_SERVER_ERROR {
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
            let body: Value = serde_json::from_slice(&bytes)?;
            assert_eq!(body["code"], "Internal");
            assert!(
                body.get("profiles").is_none(),
                "faults cannot become partial lists"
            );
        }
    }
    database.close().await?;
    Ok(())
}

#[tokio::test]
async fn template_collection_requires_read_permission_and_represents_empty_inventory() -> TestResult
{
    let (app, _, _) = common::build_app_with_scan().await;
    for (scope, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("fleet.write"), StatusCode::FORBIDDEN),
        (Some("template.read"), StatusCode::OK),
    ] {
        let mut request = Request::builder().uri(COLLECTION);
        if let Some(scope) = scope {
            request = request.header("authorization", common::oidc::bearer(scope));
        }
        let response = app.clone().oneshot(request.body(Body::empty())?).await?;
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes)?,
                json!({"profiles": []})
            );
        }
    }
    Ok(())
}
