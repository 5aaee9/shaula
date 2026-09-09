use crate::common;
#[path = "template_source_replay.rs"]
mod replay;
use crate::template_updates as support;

use axum::http::{HeaderValue, StatusCode};
use shaula_core::registry::{ControlPlaneStore, TemplateSource};
use tower::ServiceExt;

use support::{Fixture, TestResult, PROFILE};

async fn catalog(fixture: &Fixture, keys: &[&str], digest: &str) -> TestResult {
    let sources = keys
        .iter()
        .map(|key| TemplateSource {
            key: (*key).into(),
            artifact_digest: digest.into(),
            engine_ref: "terraform".into(),
            platform: "kubernetes".into(),
        })
        .collect::<Vec<_>>();
    fixture
        .store
        .store()
        .template_sources_replace(&sources, 1_800_000_004_000)
        .await?;
    Ok(())
}

fn associated_body(fixture: &Fixture, source_key: &str) -> serde_json::Value {
    let mut body = fixture.body();
    body["source_key"] = source_key.into();
    body
}

async fn put(
    fixture: &Fixture,
    body: serde_json::Value,
    etag: &str,
    idem: Option<&str>,
) -> TestResult<(StatusCode, String, String)> {
    let mut request = common::authorized("PUT", PROFILE, Some(body.to_string()));
    request.headers_mut().remove("if-none-match");
    request
        .headers_mut()
        .insert("if-match", HeaderValue::from_str(etag)?);
    if let Some(idem) = idem {
        request
            .headers_mut()
            .insert("idempotency-key", HeaderValue::from_str(idem)?);
    }
    support::response(fixture.app.clone().oneshot(request).await?).await
}

fn put_body(digest: &str, source_key: Option<&str>) -> TestResult<serde_json::Value> {
    let mut body: serde_json::Value =
        serde_json::from_str(&common::TEMPLATE_PUT_BODY.replace("PLACEHOLDER", digest))?;
    if let Some(source_key) = source_key {
        body["source_key"] = source_key.into();
    }
    Ok(body)
}

#[tokio::test]
async fn default_publication_persists_source_and_reads_it_after_reopening_database() -> TestResult {
    let fixture = Fixture::new().await?;
    catalog(&fixture, &["kubernetes"], &fixture.base_digest).await?;
    let body = put_body(&fixture.base_digest, Some("kubernetes"))?;
    let create = common::authorized(
        "PUT",
        "/api/v1/template-profiles/default",
        Some(body.to_string()),
    );
    assert_eq!(
        fixture.app.clone().oneshot(create).await?.status(),
        StatusCode::ACCEPTED
    );
    let view = common::get_json(
        &fixture.app,
        "/api/v1/template-profiles/default/revisions/1",
    )
    .await;
    assert_eq!(view["sourceKey"], "kubernetes");
    assert!(!view.to_string().contains("secret-kubeconfig"));
    let legacy = common::get_json(&fixture.app, &format!("{PROFILE}/revisions/1")).await;
    assert_eq!(legacy.get("sourceKey"), Some(&serde_json::Value::Null));
    let root = fixture
        .engine_binary
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("fixture root missing")?;
    let reopened = shaula_store::Store::open(&root.join("data/test.db")).await?;
    let reopened =
        shaula_store::registry_impl::SqliteControlPlane::new(reopened, root.join("artifacts"));
    assert_eq!(
        reopened
            .template_revision_get("default", 1)
            .await?
            .ok_or("revision missing")?
            .source_key
            .as_deref(),
        Some("kubernetes")
    );
    Ok(())
}

#[tokio::test]
async fn source_association_is_part_of_revision_equality_and_can_be_removed() -> TestResult {
    let fixture = Fixture::new().await?;
    catalog(&fixture, &["kubernetes", "other"], &fixture.base_digest).await?;
    let mut body = associated_body(&fixture, "kubernetes");
    body["artifact_digest"] = fixture.base_digest.clone().into();
    let first = fixture
        .send(body.clone(), Some(&fixture.etag), None)
        .await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    assert_eq!(
        fixture.revision(2).await?.source_key.as_deref(),
        Some("kubernetes")
    );
    let same = fixture.send(body, Some(&first.2), None).await?;
    assert_eq!(same.0, StatusCode::OK);
    let unassociated = put_body(&fixture.base_digest, None)?;
    let removed = put(&fixture, unassociated, &first.2, None).await?;
    assert_eq!(removed.0, StatusCode::ACCEPTED);
    assert_eq!(fixture.revision(3).await?.source_key, None);
    let reassociated = put(
        &fixture,
        put_body(&fixture.base_digest, Some("other"))?,
        &removed.2,
        None,
    )
    .await?;
    assert_eq!(reassociated.0, StatusCode::ACCEPTED);
    assert_eq!(
        fixture.revision(4).await?.source_key.as_deref(),
        Some("other")
    );
    let mut independent = fixture.body();
    independent["source_key"] = serde_json::Value::Null;
    let independent = fixture
        .send(independent, Some(&reassociated.2), None)
        .await?;
    assert_eq!(independent.0, StatusCode::ACCEPTED);
    assert_eq!(fixture.revision(5).await?.source_key, None);
    assert_eq!(
        fixture.revision(2).await?.source_key.as_deref(),
        Some("kubernetes")
    );
    assert_eq!(
        fixture
            .store
            .fleet_revision_latest("build")
            .await?
            .ok_or("fleet missing")?
            .template_revision,
        Some(1)
    );
    Ok(())
}

#[tokio::test]
async fn update_follows_its_source_after_catalog_upgrade_and_retains_secrets() -> TestResult {
    let fixture = Fixture::new().await?;
    catalog(&fixture, &["kubernetes"], &fixture.base_digest).await?;
    let first = put(
        &fixture,
        put_body(&fixture.base_digest, Some("kubernetes"))?,
        &fixture.etag,
        None,
    )
    .await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    let old = fixture
        .store
        .template_protected_bindings("k8s-linux", 2)
        .await?;
    catalog(&fixture, &["kubernetes"], &fixture.target_digest).await?;
    let updated = fixture
        .send(
            associated_body(&fixture, "kubernetes"),
            Some(&first.2),
            None,
        )
        .await?;
    assert_eq!(updated.0, StatusCode::ACCEPTED);
    assert_eq!(
        fixture.revision(3).await?.source_key.as_deref(),
        Some("kubernetes")
    );
    assert_eq!(
        fixture.revision(3).await?.artifact_digest,
        fixture.target_digest
    );
    assert_eq!(
        fixture.revision(2).await?.artifact_digest,
        fixture.base_digest
    );
    let new = fixture
        .store
        .template_protected_bindings("k8s-linux", 3)
        .await?;
    assert_eq!(
        old.as_ref().map(|pair| &pair.0),
        new.as_ref().map(|pair| &pair.0)
    );
    catalog(&fixture, &[], &fixture.target_digest).await?;
    assert_eq!(
        common::get_json(&fixture.app, &format!("{PROFILE}/revisions/3")).await["sourceKey"],
        "kubernetes"
    );
    Ok(())
}

#[tokio::test]
async fn source_validation_rejects_spoofed_keys_and_every_target_mismatch() -> TestResult {
    let fixture = Fixture::new().await?;
    catalog(&fixture, &["kubernetes"], &fixture.target_digest).await?;
    for (field, value) in [
        ("source_key", "missing"),
        ("source_key", ""),
        ("artifact_digest", fixture.base_digest.as_str()),
        ("engine_ref", "tofu"),
    ] {
        let mut body = associated_body(&fixture, "kubernetes");
        body[field] = value.into();
        assert_eq!(
            fixture.send(body, Some(&fixture.etag), None).await?.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    fixture
        .store
        .store()
        .template_sources_replace(
            &[TemplateSource {
                key: "kubernetes".into(),
                artifact_digest: fixture.target_digest.clone(),
                engine_ref: "terraform".into(),
                platform: "docker".into(),
            }],
            1_800_000_005_000,
        )
        .await?;
    assert_eq!(
        fixture
            .send(
                associated_body(&fixture, "kubernetes"),
                Some(&fixture.etag),
                None
            )
            .await?
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let forged = put_body(&fixture.target_digest, Some("kubernetes"))?;
    assert_eq!(
        put(&fixture, forged, &fixture.etag, None).await?.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for invalid in [serde_json::json!(7), serde_json::json!({})] {
        let mut body = fixture.body();
        body["source_key"] = invalid;
        assert_eq!(
            fixture.send(body, Some(&fixture.etag), None).await?.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 2)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn update_cannot_reassociate_to_another_source_but_ordinary_publication_can() -> TestResult {
    let fixture = Fixture::new().await?;
    catalog(&fixture, &["kubernetes", "other"], &fixture.target_digest).await?;
    let first = fixture
        .send(
            associated_body(&fixture, "kubernetes"),
            Some(&fixture.etag),
            Some("associate"),
        )
        .await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    let cross_source = fixture
        .send(associated_body(&fixture, "other"), Some(&first.2), None)
        .await?;
    assert_eq!(cross_source.0, StatusCode::UNPROCESSABLE_ENTITY);
    let ordinary = put(
        &fixture,
        put_body(&fixture.target_digest, Some("other"))?,
        &first.2,
        None,
    )
    .await?;
    assert_eq!(ordinary.0, StatusCode::ACCEPTED);
    assert_eq!(
        fixture.revision(3).await?.source_key.as_deref(),
        Some("other")
    );
    assert_eq!(
        fixture.revision(2).await?.source_key.as_deref(),
        Some("kubernetes")
    );
    Ok(())
}
