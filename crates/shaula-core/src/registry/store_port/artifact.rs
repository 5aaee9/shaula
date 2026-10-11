//! Published artifact reads (manifests, schemas, lock files).

use async_trait::async_trait;

use crate::error::CoreResult;

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Production restores missing cache material from the DB before any
    /// read/effect. Filesystem-only adapters with no external cache
    /// authority implement this as a no-op.
    async fn ensure_artifact_cached(&self, digest: &str) -> CoreResult<()>;
    async fn artifact_manifest(&self, digest: &str) -> CoreResult<Option<String>>;
    /// The artifact's declared bindings schema document (JSON Schema) —
    /// the per-field `sensitive` authority for the spec 0038 read
    /// projection and Update merge. `Ok(None)` when the artifact is not
    /// published (or its schema file is absent — callers fail closed to
    /// all-sensitive); any other read failure is an `Err`.
    async fn artifact_bindings_schema(&self, digest: &str) -> CoreResult<Option<String>>;
    /// The artifact's declared parameter schema (JSON Schema document) —
    /// the required/type/alias authority for Fleet inputs (spec 0002 §4.1,
    /// 0004 §3). A read failure is an Err (fail closed), never a silent
    /// "no schema" degradation; an artifact without declared parameters
    /// stores an empty schema document.
    async fn artifact_parameter_schema(&self, digest: &str) -> CoreResult<String>;
    async fn artifact_shape_ok(&self, digest: &str) -> CoreResult<bool>;
    /// SHA-256 of the dependency lock file inside the published artifact.
    async fn artifact_lock_digest(&self, digest: &str) -> CoreResult<Option<String>>;
    /// The dependency lock file TEXT inside the published artifact — the
    /// authority the registry recomputes the attestation provider set
    /// from (spec 0005 §5.1).
    async fn artifact_lock_file(&self, digest: &str) -> CoreResult<Option<String>>;
}
