//! Artifact-directory lookups backing the `ControlPlaneStore` port, split to
//! keep files within 400 lines. The directory layout itself is owned by
//! `shaula_core::artifact_layout` — the single authority shared by every
//! reader of the store.

use std::path::Path;

use shaula_core::artifact_layout::artifact_dir;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};

/// Reads one published file. `Ok(None)` means NOT PUBLISHED (absent
/// file — admission classifies that as "digest is not published"); a
/// PRESENT-but-unreadable file is a retryable STORAGE error (R7-03),
/// never a silent `None` that a verifier could mistake for a verdict.
fn read_published(artifact_root: &Path, digest: &str, file: &str) -> CoreResult<Option<String>> {
    let Some(dir) = artifact_dir(artifact_root, digest) else {
        return Ok(None);
    };
    match std::fs::read_to_string(dir.join(file)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::new(
            ReasonCode::StorageUnavailable,
            format!("published artifact {file} unreadable: {e}"),
        )),
    }
}

/// Published manifest text; `Ok(None)` when the artifact is not
/// published.
pub(crate) fn manifest(artifact_root: &Path, digest: &str) -> CoreResult<Option<String>> {
    read_published(artifact_root, digest, "profile.yaml")
}

/// The artifact's declared parameter schema document (JSON Schema).
/// Read failures are ERRORS (fail closed at admission), never a silent
/// "no schema" path; an artifact without declared parameters stores an
/// empty document. `Ok(None)` only for a malformed digest.
pub(crate) fn parameter_schema(artifact_root: &Path, digest: &str) -> CoreResult<Option<String>> {
    let Some(dir) = artifact_dir(artifact_root, digest) else {
        return Ok(None);
    };
    std::fs::read_to_string(dir.join("schemas/parameters.schema.json"))
        .map(Some)
        .map_err(|e| {
            CoreError::new(
                ReasonCode::StorageUnavailable,
                format!("artifact parameter schema unreadable: {e}"),
            )
        })
}

/// The three published entries every Template Candidate requires. The
/// parameter schema document is mandatory so input admission can never
/// degrade to "no schema".
pub(crate) fn shape_ok(artifact_root: &Path, digest: &str) -> bool {
    let Some(dir) = artifact_dir(artifact_root, digest) else {
        return false;
    };
    dir.join("profile.yaml").is_file()
        && dir.join(".terraform.lock.hcl").is_file()
        && dir.join("schemas").is_dir()
        && dir.join("schemas/parameters.schema.json").is_file()
}

/// Digest of the published Terraform lock file; `Ok(None)` when the
/// artifact is not published (R7-03: unreadable is an error, not
/// `None`).
pub(crate) fn lock_digest(artifact_root: &Path, digest: &str) -> CoreResult<Option<String>> {
    let Some(dir) = artifact_dir(artifact_root, digest) else {
        return Ok(None);
    };
    match std::fs::read(dir.join(".terraform.lock.hcl")) {
        Ok(bytes) => Ok(Some(sha256_hex(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::new(
            ReasonCode::StorageUnavailable,
            format!("published dependency lock unreadable: {e}"),
        )),
    }
}

/// The published Terraform lock file TEXT — the attestation provider-set
/// authority; `Ok(None)` when the artifact is not published.
pub(crate) fn lock_file(artifact_root: &Path, digest: &str) -> CoreResult<Option<String>> {
    read_published(artifact_root, digest, ".terraform.lock.hcl")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn malformed_digest_is_none_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(manifest(tmp.path(), "not-a-digest").unwrap().is_none());
        assert!(lock_file(tmp.path(), "not-a-digest").unwrap().is_none());
        assert!(lock_digest(tmp.path(), "not-a-digest").unwrap().is_none());
    }

    #[test]
    fn absent_artifact_is_none_but_unreadable_file_is_a_storage_error() {
        // R7-03: an absent artifact is "not published"; a present-but
        // unreadable authority must never degrade to that same None.
        let tmp = tempfile::tempdir().unwrap();
        let digest = format!("sha256:{}", "ab".repeat(32));
        assert!(manifest(tmp.path(), &digest).unwrap().is_none());
        let dir = artifact_dir(tmp.path(), &digest).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("profile.yaml"), "api_version: x\n").unwrap();
        #[cfg(windows)]
        {
            // A directory where the file belongs makes reads fail with an
            // OS error that is NOT NotFound.
            std::fs::create_dir(dir.join(".terraform.lock.hcl")).unwrap();
            assert!(lock_file(tmp.path(), &digest).is_err());
            assert!(lock_digest(tmp.path(), &digest).is_err());
        }
        #[cfg(not(windows))]
        {
            std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o000))
                .unwrap();
            assert!(lock_file(tmp.path(), &digest).is_err());
        }
    }
}
