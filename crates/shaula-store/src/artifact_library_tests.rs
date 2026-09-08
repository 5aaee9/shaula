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
async fn source_identity_preserves_first_import_and_requires_a_durable_archive() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Store::open(&temp.path().join("shaula.db")).await?;
    store.migrate().await?;
    let bytes = b"first source bytes";
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    let first = TemplateSource {
        key: "docker".into(),
        artifact_digest: digest.clone(),
        platform: "docker".into(),
        engine_ref: "terraform".into(),
    };
    assert!(store.template_source_insert(&first, 1).await.is_err());
    store.artifact_archive_put(&digest, bytes, 1).await?;
    store.template_source_insert(&first, 1).await?;
    let bytes = b"updated source bytes";
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    store.artifact_archive_put(&digest, bytes, 2).await?;
    let second = TemplateSource {
        artifact_digest: digest,
        ..first.clone()
    };
    store.template_source_insert(&second, 2).await?;
    assert_eq!(store.template_sources().await?, vec![first]);
    assert!(store.template_source_exists("docker").await?);
    store.migrate().await?;
    assert_eq!(store.template_sources().await?.len(), 1);
    Ok(())
}
