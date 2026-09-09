use super::*;
use sha2::{Digest, Sha256};
use shaula_core::registry::ControlPlaneStore;
use shaula_store::registry_impl::SqliteControlPlane;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn bundled() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/docker")
}

fn fixture_source(root: &Path) -> CoreResult<PathBuf> {
    let dir = root.join("defaults/docker");
    std::fs::create_dir_all(&dir).map_err(storage)?;
    let (bytes, _) = import::package(&bundled())?;
    shaula_template::artifact::extract_tar_gz(&bytes, &dir, MAX_EXPANSION)?;
    Ok(root.join("defaults"))
}

async fn adapter(root: &Path) -> CoreResult<(Store, Arc<DbArtifactPublisher>)> {
    let store = Store::open(&root.join("shaula.db"))
        .await
        .map_err(storage)?;
    store.migrate().await.map_err(storage)?;
    let cache = root.join("artifacts");
    std::fs::create_dir_all(&cache).map_err(storage)?;
    let library = Arc::new(DbArtifactPublisher::new(store.clone(), cache));
    Ok((store, library))
}

#[path = "artifact_library_sync_tests.rs"]
mod sync;

#[tokio::test]
async fn rejected_uploads_leave_neither_database_nor_usable_cache_artifacts() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let (store, library) = adapter(temp.path()).await?;
    std::fs::write(
        defaults.join("docker/main.tf"),
        "variable \"wrong\" { type = string }",
    )?;
    let (bytes, _) = import::package(&defaults.join("docker"))?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    assert!(library.publish(&bytes, &digest).await.is_err());
    assert!(store.artifact_archive_get(&digest).await?.is_none());
    let path =
        shaula_core::artifact_layout::artifact_dir(&library.root, &digest).ok_or("bad digest")?;
    assert!(!path.exists());
    assert!(!path.with_extension("tar.gz").exists());
    let (bytes, _) = import::package(&bundled())?;
    assert!(library.publish(&bytes, &digest).await.is_err());
    assert!(store.artifact_archive_digests().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn legacy_original_archives_import_without_repacking_and_unregistered_files_are_not_authority(
) -> TestResult {
    let temp = tempfile::tempdir()?;
    let (store, library) = adapter(temp.path()).await?;
    let (bytes, _) = import::package(&bundled())?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    shaula_template::ArtifactStore::new(&library.root).publish(&bytes, &digest)?;
    let plane = SqliteControlPlane::new(store.clone(), library.root.clone())
        .with_artifact_cache(library.clone());
    assert!(plane.artifact_manifest(&digest).await?.is_none());
    library.initialize(&[], 1).await?;
    assert_eq!(store.artifact_archive_get(&digest).await?, Some(bytes));
    assert!(plane.artifact_manifest(&digest).await?.is_some());
    assert!(library.sources().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn legacy_container_upload_is_rejected_but_original_archive_still_recovers() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let path = defaults.join("docker/profile.yaml");
    let mut manifest: shaula_core::template::ProfileManifest =
        serde_yaml::from_str(&std::fs::read_to_string(&path)?)?;
    manifest.container_bootstrap_contract = None;
    manifest.runner_image_digests = vec![format!(
        "registry.test/custom:old@sha256:{}",
        "a".repeat(64)
    )];
    std::fs::write(path, serde_yaml::to_string(&manifest)?)?;
    let (bytes, _) = import::package(&defaults.join("docker"))?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    let (store, library) = adapter(temp.path()).await?;
    assert!(library.publish(&bytes, &digest).await.is_err());
    assert!(library.sync_defaults(&[defaults], 1).await.is_err());
    assert!(store.artifact_archive_get(&digest).await?.is_none());
    assert!(library.sources().await?.is_empty());

    // Upgrade recovery imports the exact old bytes; it must not reinterpret
    // cleanup data as a request to publish or launch another legacy Runner.
    let published = shaula_template::ArtifactStore::new(&library.root).publish(&bytes, &digest)?;
    library.initialize(&[], 2).await?;
    assert_eq!(
        store.artifact_archive_get(&digest).await?,
        Some(bytes.clone())
    );
    std::fs::remove_dir_all(&published.final_path)?;
    assert!(library.ensure_cached(&digest).await?);
    assert_eq!(
        std::fs::read(published.final_path.with_extension("tar.gz"))?,
        bytes
    );
    Ok(())
}

#[tokio::test]
async fn corrupted_existing_cache_and_legacy_archive_fail_closed() -> TestResult {
    let temp = tempfile::tempdir()?;
    let (store, library) = adapter(temp.path()).await?;
    let (bytes, _) = import::package(&bundled())?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    library.publish(&bytes, &digest).await?;
    let cached =
        shaula_core::artifact_layout::artifact_dir(&library.root, &digest).ok_or("bad digest")?;
    std::fs::write(cached.join("schemas/parameters.schema.json"), "{}")?;
    let plane = SqliteControlPlane::new(store.clone(), library.root.clone())
        .with_artifact_cache(library.clone());
    assert!(plane.artifact_parameter_schema(&digest).await.is_err());
    assert!(store.artifact_archive_get(&digest).await?.is_some());

    let other = tempfile::tempdir()?;
    let (empty_store, other_library) = adapter(other.path()).await?;
    let published =
        shaula_template::ArtifactStore::new(&other_library.root).publish(&bytes, &digest)?;
    std::fs::write(published.final_path.with_extension("tar.gz"), b"corrupt")?;
    assert!(other_library.initialize(&[], 1).await.is_err());
    assert!(empty_store.artifact_archive_digests().await?.is_empty());
    Ok(())
}

#[test]
fn default_pack_is_deterministic_and_excludes_image_build_material() -> TestResult {
    let (first, _) = import::package(&bundled())?;
    let (second, _) = import::package(&bundled())?;
    assert_eq!(first, second);
    let temp = tempfile::tempdir()?;
    shaula_template::artifact::extract_tar_gz(&first, temp.path(), MAX_EXPANSION)?;
    assert!(temp.path().join(".terraform.lock.hcl").is_file());
    assert!(!temp.path().join("image").exists());
    Ok(())
}

#[test]
fn default_pack_never_imports_nested_repository_state_or_credentials() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let directory = defaults.join("docker");
    let (before, _) = import::package(&directory)?;
    for relative in [
        "bootstrap.sh",
        "bootstrap.cmd",
        "private.tfvars",
        "terraform.tfstate",
        "schemas/.git/config",
        "schemas/private.tfvars",
        "schemas/terraform.tfstate",
        "schemas/credentials.json",
        "schemas/bootstrap.tftpl",
        "nested/bootstrap.tftpl",
        "image/private.json",
    ] {
        let path = directory.join(relative);
        std::fs::create_dir_all(path.parent().ok_or("parent missing")?)?;
        std::fs::write(path, "secret-never-import-this-value")?;
    }
    let (after, _) = import::package(&directory)?;
    assert_eq!(before, after);
    Ok(())
}

#[tokio::test]
async fn corrupt_database_archive_is_storage_failure_for_discovery_and_runtime() -> TestResult {
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};
    let temp = tempfile::tempdir()?;
    let (_store, library) = adapter(temp.path()).await?;
    let (bytes, _) = import::package(&bundled())?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    library.publish(&bytes, &digest).await?;
    let url = format!(
        "sqlite://{}?mode=rw",
        temp.path()
            .join("shaula.db")
            .to_string_lossy()
            .replace('\\', "/")
    );
    let database = Database::connect(url).await?;
    database
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE artifact_archives SET archive = ? WHERE digest = ?",
            vec![b"corrupt".to_vec().into(), digest.clone().into()],
        ))
        .await?;
    assert!(matches!(
        library.variables(&digest).await,
        Err(CoreError {
            code: ReasonCode::StorageUnavailable,
            ..
        })
    ));
    assert!(matches!(
        library.ensure_cached(&digest).await,
        Err(CoreError {
            code: ReasonCode::StorageUnavailable,
            ..
        })
    ));
    Ok(())
}

#[tokio::test]
async fn cache_write_failure_preserves_committed_archive_for_retry() -> TestResult {
    let temp = tempfile::tempdir()?;
    let (store, library) = adapter(temp.path()).await?;
    let (bytes, _) = import::package(&bundled())?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    let cached =
        shaula_core::artifact_layout::artifact_dir(&library.root, &digest).ok_or("bad digest")?;
    let prefix = cached.parent().ok_or("cache prefix missing")?;
    std::fs::write(prefix, b"not a directory")?;
    assert!(matches!(
        library.publish(&bytes, &digest).await,
        Err(CoreError {
            code: ReasonCode::StorageUnavailable,
            ..
        })
    ));
    assert_eq!(
        store.artifact_archive_get(&digest).await?.as_deref(),
        Some(bytes.as_slice())
    );
    std::fs::remove_file(prefix)?;
    library.publish(&bytes, &digest).await?;
    assert!(cached.join("main.tf").is_file());
    Ok(())
}

#[cfg(unix)]
#[test]
fn default_pack_rejects_redirected_render_templates() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let outside = temp.path().join("outside.tftpl");
    std::fs::write(&outside, "outside render template")?;
    std::os::unix::fs::symlink(&outside, defaults.join("docker/bootstrap.tftpl"))?;
    assert!(import::package(&defaults.join("docker")).is_err());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn redirected_source_roots_and_cache_prefixes_are_rejected() -> TestResult {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let (store, library) = adapter(temp.path()).await?;
    let redirect = temp.path().join("linked-defaults");
    symlink(&defaults, &redirect)?;
    assert!(library.initialize(&[redirect], 1).await.is_err());
    assert!(library.sources().await?.is_empty());
    let (bytes, _) = import::package(&bundled())?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    store.artifact_archive_put(&digest, &bytes, 1).await?;
    let cached =
        shaula_core::artifact_layout::artifact_dir(&library.root, &digest).ok_or("bad digest")?;
    let outside = temp.path().join("outside-cache");
    std::fs::create_dir(&outside)?;
    symlink(&outside, cached.parent().ok_or("cache prefix missing")?)?;
    assert!(matches!(
        library.ensure_cached(&digest).await,
        Err(CoreError {
            code: ReasonCode::StorageUnavailable,
            ..
        })
    ));
    assert_eq!(std::fs::read_dir(outside)?.count(), 0);
    Ok(())
}
