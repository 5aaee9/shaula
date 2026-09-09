use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn archive_content_is_digest_bound_and_concurrent_insert_is_idempotent() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("shaula.db")).await?;
    store.migrate().await?;
    let bytes = b"opaque validated archive fixture";
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    let (a, b) = tokio::join!(
        store.artifact_archive_put(&digest, bytes, 1),
        store.artifact_archive_put(&digest, bytes, 2)
    );
    a?;
    b?;
    assert_eq!(
        store.artifact_archive_digests().await?,
        vec![digest.clone()]
    );
    assert_eq!(
        store.artifact_archive_get(&digest).await?.as_deref(),
        Some(bytes.as_slice())
    );
    assert!(store
        .artifact_archive_put(&digest, b"different bytes", 3)
        .await
        .is_err());
    assert_eq!(
        store.artifact_archive_get(&digest).await?.as_deref(),
        Some(bytes.as_slice())
    );
    Ok(())
}

#[tokio::test]
async fn source_catalog_replaces_content_and_metadata_under_the_same_key() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("shaula.db")).await?;
    store.migrate().await?;
    let first = archived_source(&store, "docker", b"first source bytes").await?;
    store
        .template_sources_replace(std::slice::from_ref(&first), 1)
        .await?;
    let second = TemplateSource {
        platform: "kubernetes".into(),
        engine_ref: "opentofu".into(),
        ..archived_source(&store, "docker", b"updated source bytes").await?
    };
    store
        .template_sources_replace(std::slice::from_ref(&second), 2)
        .await?;
    assert_eq!(store.template_sources().await?, vec![second.clone()]);
    assert_eq!(
        store.artifact_archive_get(&first.artifact_digest).await?,
        Some(b"first source bytes".to_vec())
    );
    store.migrate().await?;
    assert_eq!(store.template_sources().await?, vec![second]);
    Ok(())
}

#[tokio::test]
async fn source_catalog_removes_aliases_absent_from_current_sources() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("shaula.db")).await?;
    store.migrate().await?;
    let old = archived_source(&store, "docker", b"old source").await?;
    let official = archived_source(&store, "docker-official-2-337-0", b"official source").await?;
    store
        .template_sources_replace(&[old, official.clone()], 1)
        .await?;
    let current = TemplateSource {
        key: "docker".into(),
        ..official
    };
    store
        .template_sources_replace(std::slice::from_ref(&current), 2)
        .await?;
    assert_eq!(store.template_sources().await?, vec![current]);
    assert_eq!(store.artifact_archive_digests().await?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn empty_source_catalog_preserves_archives_and_published_revisions() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("shaula.db")).await?;
    store.migrate().await?;
    let first = archived_source(&store, "docker", b"docker source").await?;
    let second = archived_source(&store, "kubernetes", b"kubernetes source").await?;
    let tx = store.begin().await?;
    store
        .template_commit_revision(
            &tx,
            shaula_core::template::TemplateRevisionInsert {
                key: "local-docker".into(),
                incarnation: "source-preservation-test".into(),
                revision: 1,
                artifact_digest: first.artifact_digest.clone(),
                engine_ref: first.engine_ref.clone(),
                bindings_json: Some(r#"{"docker_host":"unix:///var/run/docker.sock"}"#.into()),
                bindings_digest: Some("original-bindings-digest".into()),
                fleet_input_policy_json: Some("{}".into()),
            },
            1,
        )
        .await?;
    tx.commit().await?;
    let original_profile = store.template_profile_get("local-docker").await?;
    let original_revision = store.template_revision_get("local-docker", 1).await?;
    store.template_sources_replace(&[first, second], 1).await?;
    let original_archives = archives::Entity::find()
        .order_by_asc(archives::Column::Digest)
        .all(store.connection())
        .await?;
    store.template_sources_replace(&[], 2).await?;
    assert!(store.template_sources().await?.is_empty());
    assert_eq!(
        archives::Entity::find()
            .order_by_asc(archives::Column::Digest)
            .all(store.connection())
            .await?,
        original_archives
    );
    assert_eq!(
        store.template_profile_get("local-docker").await?,
        original_profile
    );
    assert_eq!(
        store.template_revision_get("local-docker", 1).await?,
        original_revision
    );
    Ok(())
}

#[tokio::test]
async fn source_catalog_missing_archive_rolls_back_the_entire_replacement() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("shaula.db")).await?;
    store.migrate().await?;
    let first = archived_source(&store, "docker", b"old source").await?;
    let alias = TemplateSource {
        key: "docker-legacy".into(),
        ..first.clone()
    };
    let original = vec![first, alias];
    store.template_sources_replace(&original, 1).await?;
    let valid = archived_source(&store, "docker", b"new source").await?;
    let missing = TemplateSource {
        key: "kubernetes".into(),
        artifact_digest: format!("sha256:{}", hex::encode(Sha256::digest(b"missing"))),
        ..valid.clone()
    };
    assert!(store
        .template_sources_replace(&[valid, missing], 2)
        .await
        .is_err());
    assert_eq!(store.template_sources().await?, original);
    Ok(())
}

#[tokio::test]
async fn source_catalog_duplicate_key_rolls_back_the_entire_replacement() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("shaula.db")).await?;
    store.migrate().await?;
    let first = archived_source(&store, "docker", b"old source").await?;
    store
        .template_sources_replace(std::slice::from_ref(&first), 1)
        .await?;
    let replacement = archived_source(&store, "docker", b"new source").await?;
    assert!(store
        .template_sources_replace(&[replacement.clone(), replacement], 2)
        .await
        .is_err());
    assert_eq!(store.template_sources().await?, vec![first]);
    Ok(())
}

async fn archived_source(
    store: &Store,
    key: &str,
    bytes: &[u8],
) -> Result<TemplateSource, Box<dyn std::error::Error>> {
    let artifact_digest = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    store
        .artifact_archive_put(&artifact_digest, bytes, 1)
        .await?;
    Ok(TemplateSource {
        key: key.into(),
        artifact_digest,
        platform: "docker".into(),
        engine_ref: "terraform".into(),
    })
}
