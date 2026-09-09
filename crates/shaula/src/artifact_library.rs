//! Composition adapter for durable templates and their derived filesystem cache.

#[path = "artifact_library_import.rs"]
mod import;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::template_library::ArtifactCache;
use shaula_core::registry::{TemplateSource, TemplateVariables};
use shaula_http::router::ArtifactPublisher;
use shaula_store::Store;
use std::path::PathBuf;

const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_EXPANSION: u64 = 256 * 1024 * 1024;

pub(crate) struct DbArtifactPublisher {
    store: Store,
    root: PathBuf,
    /// Extraction/publication is serialized so two publishers cannot race the
    /// same filesystem cache; DB unique constraints independently fence writes.
    cache_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl DbArtifactPublisher {
    pub(crate) fn new(store: Store, root: PathBuf) -> Self {
        Self {
            store,
            root,
            cache_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) async fn initialize(&self, defaults: &[PathBuf], now: i64) -> CoreResult<()> {
        self.import_legacy(now).await?;
        for digest in self
            .store
            .artifact_archive_digests()
            .await
            .map_err(storage)?
        {
            self.ensure_cached(&digest).await?;
        }
        for digest in self
            .store
            .template_referenced_artifacts()
            .await
            .map_err(storage)?
        {
            if !self.ensure_cached(&digest).await? {
                return Err(CoreError::new(ReasonCode::StorageUnavailable,
                    "referenced template archive is missing; restore its original digest-addressed archive"));
            }
        }
        self.sync_defaults(defaults, now).await?;
        Ok(())
    }

    async fn save(
        &self,
        bytes: Vec<u8>,
        digest: String,
        now: i64,
        validate_variables: bool,
    ) -> CoreResult<u64> {
        let size =
            u64::try_from(bytes.len()).map_err(|_| invalid("artifact exceeds body limit"))?;
        if size > MAX_ARCHIVE {
            return Err(invalid("artifact exceeds body limit"));
        }
        // Validate in an isolated temporary directory. Rejected uploads never
        // leave a filesystem artifact that Profile admission could mistake for DB data.
        let (bytes, digest) = tokio::task::spawn_blocking(move || {
            validate_archive(&bytes, &digest, validate_variables)?;
            Ok::<_, CoreError>((bytes, digest))
        })
        .await
        .map_err(|_| storage("artifact validation task failed"))??;
        self.store
            .artifact_archive_put(&digest, &bytes, now)
            .await
            .map_err(storage)?;
        // A cache failure after the commit is retryable: the immutable archive
        // remains in SQLite and the same digest can be restored on retry/restart.
        self.ensure_cached(&digest).await?;
        Ok(size)
    }
}

fn validate_archive(bytes: &[u8], digest: &str, validate_variables: bool) -> CoreResult<()> {
    shaula_template::artifact::verify_digest(bytes, digest)?;
    let temp = tempfile::tempdir().map_err(storage)?;
    shaula_template::artifact::extract_tar_gz(bytes, temp.path(), MAX_EXPANSION)?;
    let manifest = std::fs::read_to_string(temp.path().join("profile.yaml"))
        .map_err(|_| invalid("artifact missing or unreadable profile.yaml"))?;
    let manifest = shaula_template::manifest::parse_manifest(&manifest)?;
    shaula_template::manifest::verify_artifact_shape(temp.path())?;
    if validate_variables {
        // Legacy import and cache recovery retain original cleanup material.
        // Only a new upload/default source must adopt the current runner policy.
        manifest.validate_new_container_profile()?;
        shaula_template::variables::discover_variables(temp.path(), digest)?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl ArtifactCache for DbArtifactPublisher {
    async fn ensure_cached(&self, digest: &str) -> CoreResult<bool> {
        let guard = self.cache_lock.clone().lock_owned().await;
        let Some(bytes) = self
            .store
            .artifact_archive_get(digest)
            .await
            .map_err(storage)?
        else {
            return Ok(false);
        };
        let root = self.root.clone();
        let digest = digest.to_owned();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            shaula_template::artifact_cache::ensure_cached(&root, &bytes, &digest)
        })
        .await
        .map_err(|_| storage("artifact cache task failed"))??;
        Ok(true)
    }
}

#[async_trait::async_trait]
impl ArtifactPublisher for DbArtifactPublisher {
    async fn publish(&self, bytes: &[u8], digest: &str) -> CoreResult<u64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(storage)?;
        let now = i64::try_from(now.as_millis()).map_err(storage)?;
        self.save(bytes.to_vec(), digest.to_owned(), now, true)
            .await
    }

    async fn sources(&self) -> CoreResult<Vec<TemplateSource>> {
        self.store.template_sources().await.map_err(storage)
    }

    async fn variables(&self, digest: &str) -> CoreResult<Option<TemplateVariables>> {
        let Some(bytes) = self
            .store
            .artifact_archive_get(digest)
            .await
            .map_err(storage)?
        else {
            return Ok(None);
        };
        let digest = digest.to_owned();
        tokio::task::spawn_blocking(move || {
            shaula_template::artifact::verify_digest(&bytes, &digest).map_err(storage)?;
            let temp = tempfile::tempdir().map_err(storage)?;
            shaula_template::artifact::extract_tar_gz(&bytes, temp.path(), MAX_EXPANSION)
                .map_err(storage)?;
            shaula_template::variables::discover_variables(temp.path(), &digest).map(Some)
        })
        .await
        .map_err(|_| storage("variable discovery task failed"))?
    }
}

fn storage(error: impl std::fmt::Display) -> CoreError {
    CoreError::new(
        ReasonCode::StorageUnavailable,
        format!("template storage unavailable: {error}"),
    )
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, message)
}

#[cfg(test)]
#[path = "artifact_library_tests.rs"]
mod tests;
