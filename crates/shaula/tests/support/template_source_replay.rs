use axum::http::StatusCode;
use shaula_core::registry::{ControlPlaneStore, IdempotencyLookup};

use super::support::{Fixture, TestResult};
use super::{associated_body, catalog, put, put_body};

#[tokio::test]
async fn accepted_source_updates_and_noops_replay_after_catalog_changes_and_removal() -> TestResult
{
    let fixture = Fixture::new().await?;
    catalog(&fixture, &["kubernetes"], &fixture.target_digest).await?;
    let body = associated_body(&fixture, "kubernetes");
    let first = fixture
        .send(body.clone(), Some(&fixture.etag), Some("update"))
        .await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    let noop_update = fixture
        .send(body.clone(), Some(&first.2), Some("noop-update"))
        .await?;
    assert_eq!(noop_update.0, StatusCode::OK);
    let ordinary = put_body(&fixture.target_digest, Some("kubernetes"))?;
    let noop_put = put(&fixture, ordinary.clone(), &first.2, Some("noop-put")).await?;
    assert_eq!(noop_put.0, StatusCode::OK);
    for sources in [vec!["kubernetes"], vec![]] {
        catalog(&fixture, &sources, &fixture.base_digest).await?;
        assert_eq!(
            fixture
                .send(body.clone(), Some(&fixture.etag), Some("update"))
                .await?,
            first
        );
        assert_eq!(
            fixture
                .send(body.clone(), Some(&first.2), Some("noop-update"))
                .await?,
            noop_update
        );
        assert_eq!(
            put(&fixture, ordinary.clone(), &first.2, Some("noop-put")).await?,
            noop_put
        );
        // A fresh request must validate even when its content would be a NoOp.
        assert_eq!(
            fixture
                .send(body.clone(), Some(&first.2), Some("fresh"))
                .await?
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            put(&fixture, ordinary.clone(), &first.2, Some("fresh-put"))
                .await?
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 3)
        .await?
        .is_none());
    assert_eq!(
        fixture.revision(2).await?.source_key.as_deref(),
        Some("kubernetes")
    );
    Ok(())
}

#[tokio::test]
async fn changing_source_addition_removal_or_key_conflicts_with_an_existing_request() -> TestResult
{
    let fixture = Fixture::new().await?;
    catalog(&fixture, &["kubernetes", "other"], &fixture.target_digest).await?;
    let first = fixture
        .send(
            associated_body(&fixture, "kubernetes"),
            Some(&fixture.etag),
            Some("update"),
        )
        .await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    for source in [serde_json::Value::Null, serde_json::json!("other")] {
        let mut body = fixture.body();
        body["source_key"] = source;
        assert_eq!(
            fixture
                .send(body, Some(&fixture.etag), Some("update"))
                .await?
                .0,
            StatusCode::CONFLICT
        );
    }
    let ordinary = put_body(&fixture.target_digest, Some("kubernetes"))?;
    assert_eq!(
        put(&fixture, ordinary, &first.2, Some("put")).await?.0,
        StatusCode::OK
    );
    for source in [None, Some("other")] {
        assert_eq!(
            put(
                &fixture,
                put_body(&fixture.target_digest, source)?,
                &first.2,
                Some("put")
            )
            .await?
            .0,
            StatusCode::CONFLICT
        );
    }
    assert!(fixture
        .store
        .template_revision_get("k8s-linux", 3)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn absent_source_keeps_legacy_put_and_update_identities_and_null_replays() -> TestResult {
    let fixture = Fixture::new().await?;
    let ordinary = put_body(&fixture.target_digest, None)?;
    let first = put(
        &fixture,
        ordinary.clone(),
        &fixture.etag,
        Some("legacy-put"),
    )
    .await?;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    let legacy_put_canonical = format!(
        "{}|terraform|{{\"size_class\":[\"standard\"]}}",
        fixture.target_digest
    );
    assert_legacy_identity(&fixture, "legacy-put", &legacy_put_canonical).await?;
    let mut explicit_null = ordinary;
    explicit_null["source_key"] = serde_json::Value::Null;
    assert_eq!(
        put(&fixture, explicit_null, &fixture.etag, Some("legacy-put")).await?,
        first
    );
    let head = fixture
        .store
        .template_profile_get("k8s-linux")
        .await?
        .ok_or("head missing")?;
    let mut body = fixture.body();
    body["artifact_digest"] = fixture.base_digest.clone().into();
    let updated = fixture
        .send(body.clone(), Some(&first.2), Some("legacy-update"))
        .await?;
    assert_eq!(updated.0, StatusCode::ACCEPTED);
    let legacy_update_canonical = serde_json::json!({
        "operation":"template_profile_update",
        "base":[head.incarnation,2],
        "artifact_digest":fixture.base_digest,
        "engine_ref":"terraform",
        "fleet_input_policy":null,
    })
    .to_string();
    assert_legacy_identity(&fixture, "legacy-update", &legacy_update_canonical).await?;
    body["source_key"] = serde_json::Value::Null;
    assert_eq!(
        fixture
            .send(body.clone(), Some(&first.2), Some("legacy-update"))
            .await?,
        updated
    );
    body["source_key"] = "kubernetes".into();
    assert_eq!(
        fixture
            .send(body, Some(&first.2), Some("legacy-update"))
            .await?
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(fixture.revision(3).await?.source_key, None);
    Ok(())
}

async fn assert_legacy_identity(fixture: &Fixture, idem: &str, canonical: &str) -> TestResult {
    let hash = shaula_core::auth::request_hash_parts(&[
        b"template_profile",
        b"k8s-linux",
        idem.as_bytes(),
        canonical.as_bytes(),
    ]);
    assert!(matches!(
        fixture
            .store
            .idempotency_find("template_profile", "k8s-linux", idem, &hash)
            .await?,
        IdempotencyLookup::Replay(_)
    ));
    Ok(())
}
