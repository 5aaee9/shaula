//! Registry collections are keyset-paginated with an opaque, kind-bound cursor
//! (spec 0002 §5.3, spec 0005 §4).

use crate::common;

use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use serde_json::Value;
use tower::ServiceExt;

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

async fn get(
    app: &axum::Router,
    uri: &str,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    let response = app
        .clone()
        .oneshot(common::authorized("GET", uri, None))
        .await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

fn keys(body: &Value, field: &str) -> Vec<String> {
    body[field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["key"].as_str().map(str::to_owned))
        .collect()
}

/// Walks every page of `path` with `limit` and returns the keys and cursors seen.
async fn walk(
    app: &axum::Router,
    path: &str,
    field: &str,
    limit: usize,
) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error>> {
    let (mut all, mut cursors) = (Vec::new(), Vec::new());
    let mut uri = format!("{path}?limit={limit}");
    loop {
        let (status, body) = get(app, &uri).await?;
        assert_eq!(status, StatusCode::OK, "{uri}");
        let page = keys(&body, field);
        assert!(page.len() <= limit);
        all.extend(page);
        match body["next_cursor"].as_str() {
            Some(next) => {
                cursors.push(next.to_owned());
                uri = format!("{path}?limit={limit}&cursor={next}");
            }
            None => return Ok((all, cursors)),
        }
    }
}

#[tokio::test]
async fn fleet_and_template_collections_page_in_key_order() -> TestResult {
    let (app, _, _, root) = common::build_app_with_artifact_root().await;
    let database = database_for(&root).await?;
    for key in ["delta", "alpha", "charlie", "bravo", "echo"] {
        database.execute_unprepared(&format!(
            "INSERT INTO fleets(key,incarnation,desired_revision,mutation_fence,created_at,updated_at) VALUES('{key}','i-{key}',1,1,1,1)"
        )).await?;
        database.execute_unprepared(&format!(
            "INSERT INTO template_profiles (key,incarnation,desired_revision,active_revision,observed_revision,status,deletion_requested,created_at,updated_at) VALUES ('{key}','i-{key}',1,1,1,'Active',0,1,1);
            INSERT INTO template_profile_revisions (profile_key,revision,artifact_digest,engine_ref,state,created_at) VALUES ('{key}',1,'fixture-artifact','terraform','Active',1)"
        )).await?;
    }
    database.execute_unprepared(
        "INSERT INTO fleets(key,incarnation,desired_revision,mutation_fence,tombstone,created_at,updated_at) VALUES('bravo-gone','i',1,1,1,1,1)",
    ).await?;
    let expected = ["alpha", "bravo", "charlie", "delta", "echo"];
    for (path, field) in [
        ("/api/v1/fleets", "fleets"),
        ("/api/v1/template-profiles", "profiles"),
    ] {
        let (all, cursors) = walk(&app, path, field, 2).await?;
        assert_eq!(all, expected, "{path}");
        assert_eq!(cursors.len(), 2, "{path}");
        let (all, cursors) = walk(&app, path, field, 5).await?;
        assert_eq!(all, expected, "{path} exact page");
        assert!(
            cursors.is_empty(),
            "{path}: no cursor without a further row"
        );
        let (status, body) = get(&app, path).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(keys(&body, field), expected, "{path}: default page");
        assert!(body["next_cursor"].is_null());
    }
    database.close().await?;
    Ok(())
}

#[tokio::test]
async fn list_cursor_and_limit_are_validated_and_bound_to_their_collection() -> TestResult {
    let (app, _, _, root) = common::build_app_with_artifact_root().await;
    let database = database_for(&root).await?;
    for key in ["a", "b"] {
        database.execute_unprepared(&format!(
            "INSERT INTO fleets(key,incarnation,desired_revision,mutation_fence,created_at,updated_at) VALUES('{key}','i-{key}',1,1,1,1)"
        )).await?;
    }
    let (_, first) = get(&app, "/api/v1/fleets?limit=1").await?;
    let fleet_cursor = first["next_cursor"].as_str().ok_or("cursor missing")?;
    let (status, second) = get(
        &app,
        &format!("/api/v1/fleets?limit=1&cursor={fleet_cursor}"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys(&second, "fleets"), ["b"]);
    for uri in [
        "/api/v1/fleets?limit=0".to_owned(),
        "/api/v1/fleets?limit=201".to_owned(),
        "/api/v1/fleets?limit=many".to_owned(),
        "/api/v1/fleets?cursor=%21%21".to_owned(),
        "/api/v1/fleets?offset=1".to_owned(),
        format!("/api/v1/template-profiles?cursor={fleet_cursor}"),
    ] {
        let (status, body) = get(&app, &uri).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["code"], "ListQueryInvalid", "{uri}");
    }
    database.close().await?;
    Ok(())
}
