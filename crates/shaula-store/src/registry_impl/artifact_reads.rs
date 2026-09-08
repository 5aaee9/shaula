//! Database authority checks precede every derived artifact-cache read.

use super::{artifacts, SqliteControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::ControlPlaneStore;

impl SqliteControlPlane {
    pub(super) async fn artifact_manifest_impl(&self, digest: &str) -> CoreResult<Option<String>> {
        if !self.artifact_available(digest).await? {
            return Ok(None);
        }
        artifacts::manifest(&self.artifact_root, digest)
    }

    pub(super) async fn artifact_parameter_schema_impl(&self, digest: &str) -> CoreResult<String> {
        self.ensure_artifact_cached(digest).await?;
        artifacts::parameter_schema(&self.artifact_root, digest)?.ok_or_else(|| {
            CoreError::new(ReasonCode::StorageUnavailable, "artifact digest malformed")
        })
    }

    pub(super) async fn artifact_shape_impl(&self, digest: &str) -> CoreResult<bool> {
        if !self.artifact_available(digest).await? {
            return Ok(false);
        }
        Ok(artifacts::shape_ok(&self.artifact_root, digest))
    }

    pub(super) async fn artifact_lock_digest_impl(
        &self,
        digest: &str,
    ) -> CoreResult<Option<String>> {
        if !self.artifact_available(digest).await? {
            return Ok(None);
        }
        artifacts::lock_digest(&self.artifact_root, digest)
    }

    pub(super) async fn artifact_lock_file_impl(&self, digest: &str) -> CoreResult<Option<String>> {
        if !self.artifact_available(digest).await? {
            return Ok(None);
        }
        artifacts::lock_file(&self.artifact_root, digest)
    }
}
