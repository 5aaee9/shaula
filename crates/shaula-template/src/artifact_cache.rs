//! Rebuild missing executable cache material without accepting a different file tree.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use std::path::Path;

pub fn ensure_cached(root: &Path, bytes: &[u8], digest: &str) -> CoreResult<()> {
    crate::artifact::verify_digest(bytes, digest).map_err(|_| unavailable())?;
    let store = crate::ArtifactStore::new(root);
    let path = store.path_for(digest).ok_or_else(unavailable)?;
    // Never follow a redirected cache tree or archive. The data-directory lock
    // protects ownership; individual entries still must be ordinary material.
    for entry in [
        path.parent().ok_or_else(unavailable)?,
        path.as_path(),
        &path.with_extension("tar.gz"),
    ] {
        match std::fs::symlink_metadata(entry) {
            Ok(metadata) if redirected(&metadata) => return Err(unavailable()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable()),
        }
    }
    if !path.exists() || !path.with_extension("tar.gz").exists() {
        store.publish(bytes, digest).map_err(|_| unavailable())?;
    }
    let mut pending = vec![path.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|_| unavailable())?;
            if redirected(&metadata) {
                return Err(unavailable());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if !metadata.is_file() {
                return Err(unavailable());
            }
        }
    }
    // This verifies both the original archive hash and the expanded file tree;
    // existing local files never become authority merely because they exist.
    crate::artifact_integrity::material_digest(&path, digest)
        .map(|_| ())
        .map_err(|_| unavailable())
}

fn redirected(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn unavailable() -> CoreError {
    CoreError::new(
        ReasonCode::StorageUnavailable,
        "template artifact cache is unavailable or differs from its database archive",
    )
}
