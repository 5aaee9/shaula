//! Profile reads compare the recorded default source with the current catalog,
//! and the advertised target is exactly what a one-click update submits.
use crate::common;
use crate::template_updates as support;

use shaula_core::registry::TemplateSource;

use support::{Fixture, TestResult, PROFILE};

async fn catalog(fixture: &Fixture, digest: &str, platform: &str) -> TestResult {
    let sources = [TemplateSource {
        key: "kubernetes".into(),
        artifact_digest: digest.into(),
        engine_ref: "terraform".into(),
        platform: platform.into(),
    }];
    fixture
        .store
        .store()
        .template_sources_replace(&sources, 1_800_000_004_000)
        .await?;
    Ok(())
}

async fn source_update(fixture: &Fixture) -> TestResult<(serde_json::Value, serde_json::Value)> {
    let detail = common::get_json(&fixture.app, PROFILE).await;
    let list = common::get_json(&fixture.app, "/api/v1/template-profiles").await;
    let listed = list["profiles"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == "k8s-linux"))
        .ok_or("profile missing from list")?;
    assert_eq!(listed["sourceUpdate"], detail["sourceUpdate"]);
    Ok((detail["sourceUpdate"].clone(), detail))
}

#[tokio::test]
async fn source_update_tracks_catalog_and_its_target_applies_in_one_request() -> TestResult {
    let fixture = Fixture::new().await?;
    // An unassociated revision never claims a default source.
    catalog(&fixture, &fixture.base_digest, "kubernetes").await?;
    assert_eq!(source_update(&fixture).await?.0, serde_json::Value::Null);

    let link = serde_json::json!({
        "source_key": "kubernetes",
        "artifact_digest": fixture.base_digest,
        "engine_ref": "terraform",
    });
    let linked = fixture.send(link, Some(&fixture.etag), None).await?;
    assert_eq!(linked.0, axum::http::StatusCode::ACCEPTED, "{}", linked.1);
    let (update, _) = source_update(&fixture).await?;
    assert_eq!(update["sourceKey"], "kubernetes");
    assert_eq!(update["state"], "UpToDate");
    assert_eq!(update["targetDigest"], fixture.base_digest.as_str());

    // A package upgrade changes the stable source's content.
    catalog(&fixture, &fixture.target_digest, "kubernetes").await?;
    let (update, _) = source_update(&fixture).await?;
    assert_eq!(update["state"], "Available");
    assert_eq!(update["targetDigest"], fixture.target_digest.as_str());
    assert_eq!(update["targetEngine"], "terraform");

    // The one-click body is built only from the advertised read.
    let one_click = serde_json::json!({
        "source_key": update["sourceKey"],
        "artifact_digest": update["targetDigest"],
        "engine_ref": update["targetEngine"],
    });
    let applied = fixture.send(one_click, Some(&linked.2), None).await?;
    assert_eq!(applied.0, axum::http::StatusCode::ACCEPTED, "{}", applied.1);
    let row = fixture.revision(3).await?;
    assert_eq!(row.artifact_digest, fixture.target_digest);
    assert_eq!(row.source_key.as_deref(), Some("kubernetes"));
    let (update, detail) = source_update(&fixture).await?;
    assert_eq!(detail["desiredRevision"], 3);
    assert_eq!(update["state"], "UpToDate");

    // Removing the source keeps the recorded key but offers no target.
    fixture
        .store
        .store()
        .template_sources_replace(&[], 1_800_000_005_000)
        .await?;
    let (update, _) = source_update(&fixture).await?;
    assert_eq!(update["sourceKey"], "kubernetes");
    assert_eq!(update["state"], "Unavailable");
    assert_eq!(update["targetEngine"], serde_json::Value::Null);
    Ok(())
}
