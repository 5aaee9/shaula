//! Receipt-authorized cleanup. Refuse the entire tree if any path is unknown,
//! mutable evidence, redirected, or no longer backed by retained material.
use sha2::{Digest, Sha256};
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::cleanup::*,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub struct ReceiptWorkspaceReaper {
    work_root: PathBuf,
    artifact_root: PathBuf,
}

impl ReceiptWorkspaceReaper {
    pub fn new(work_root: PathBuf, artifact_root: PathBuf) -> Self {
        Self {
            work_root,
            artifact_root,
        }
    }
}

#[async_trait::async_trait]
impl WorkspaceReaper for ReceiptWorkspaceReaper {
    async fn reap(&self, request: WorkspaceCleanup) -> StateResult<CleanupOutcome> {
        let work = self.work_root.clone();
        let artifacts = self.artifact_root.clone();
        tokio::task::spawn_blocking(move || reap(&work, &artifacts, &request))
            .await
            .map_err(|_| StateError::Unavailable)?
    }
}

fn contained(root: &Path, path: &Path) -> bool {
    let Ok(root) = std::fs::canonicalize(root) else {
        return false;
    };
    let Ok(path) = std::fs::canonicalize(path) else {
        return false;
    };
    path != root && path.starts_with(root)
}

// Bounded inventory; no symlink traversal, including provider cache links.
fn inventory(
    root: &Path,
    relative: &Path,
    files: &mut BTreeMap<PathBuf, u64>,
    dirs: &mut Vec<PathBuf>,
) -> StateResult<()> {
    if files.len() + dirs.len() > 8192 || relative.components().count() > 32 {
        return Err(StateError::TooLarge);
    }
    for item in std::fs::read_dir(root.join(relative)).map_err(|_| StateError::Unavailable)? {
        let item = item.map_err(|_| StateError::Unavailable)?;
        let path = relative.join(item.file_name());
        let meta = std::fs::symlink_metadata(item.path()).map_err(|_| StateError::Unavailable)?;
        if meta.is_dir() {
            inventory(root, &path, files, dirs)?;
            dirs.push(path);
        } else if meta.is_file() {
            files.insert(path, meta.len());
        } else {
            return Err(StateError::Invalid);
        }
    }
    Ok(())
}

fn digest(path: &Path) -> StateResult<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|_| StateError::Unavailable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| StateError::Unavailable)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("sha256:{}", hex::encode(digest.finalize())))
}

fn reap(work: &Path, artifacts: &Path, request: &WorkspaceCleanup) -> StateResult<CleanupOutcome> {
    use CleanupOutcome::{Reaped, Retained};
    let workspace = &request.workspace;
    if workspace.file_name().and_then(|s| s.to_str())
        != Some(&request.receipt.claim.generation_id.to_string())
    {
        return Ok(Retained);
    }
    if !workspace
        .try_exists()
        .map_err(|_| StateError::Unavailable)?
    {
        return Ok(Reaped);
    }
    if !contained(work, workspace)
        || !contained(artifacts, &request.artifact)
        || !std::fs::symlink_metadata(workspace).is_ok_and(|m| m.is_dir())
        || crate::artifact_integrity::material_digest(&request.artifact, &request.artifact_digest)
            .is_err()
    {
        return Ok(Retained);
    }
    let mut files = BTreeMap::new();
    let mut dirs = Vec::new();
    if inventory(workspace, Path::new(""), &mut files, &mut dirs).is_err() {
        return Ok(Retained);
    }
    for (path, size) in &files {
        let name = path.to_string_lossy().replace('\\', "/");
        let allowed = match name.as_str() {
            "shaula.tfvars.json" => {
                *size <= shaula_core::state_backend::MAX_STATE_BYTES as u64
                    && request
                        .retained_input_digest
                        .as_ref()
                        .is_some_and(|expected| {
                            digest(&workspace.join(path)).is_ok_and(|actual| &actual == expected)
                        })
            }
            "shaula.backend.tf" => {
                *size < 128
                    && std::fs::read(workspace.join(path))
                        .is_ok_and(|bytes| bytes == b"terraform {\n  backend \"http\" {}\n}\n")
            }
            "tfplan" => true,
            ".terraform/terraform.tfstate" => {
                *size < 64 * 1024
                    && std::fs::read(workspace.join(path))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .is_some_and(|v| {
                            v.pointer("/backend/type").and_then(|v| v.as_str()) == Some("http")
                                && v.pointer("/backend/config")
                                    .and_then(|v| v.as_object())
                                    .is_some_and(|v| v.values().all(|v| v.is_null()))
                        })
            }
            _ if name.contains("tfstate") || name.ends_with(".tfstate.backup") => false,
            _ if name.starts_with(".terraform/providers/") => path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("terraform-provider-") || s == "LICENSE.txt"),
            _ => {
                std::fs::metadata(request.artifact.join(path))
                    .is_ok_and(|m| m.is_file() && m.len() == *size)
                    && digest(&workspace.join(path))? == digest(&request.artifact.join(path))?
            }
        };
        if !allowed {
            return Ok(Retained);
        }
    }
    // Unknown empty directories are retained too. Known directories must lead
    // to a verified file or correspond to the retained artifact/cache layout.
    for dir in &dirs {
        if !files.keys().any(|file| file.starts_with(dir))
            && !request.artifact.join(dir).is_dir()
            && dir != Path::new(".terraform")
            && !provider_cache_directory(dir)
        {
            return Ok(Retained);
        }
    }
    // Fence is established before this call. Remove only inventoried entries,
    // never recursive-delete an unchecked path; a new entry prevents rmdir.
    for path in files.keys() {
        std::fs::remove_file(workspace.join(path)).map_err(|_| StateError::Unavailable)?;
    }
    for path in dirs {
        std::fs::remove_dir(workspace.join(path)).map_err(|_| StateError::Unavailable)?;
    }
    std::fs::remove_dir(workspace).map_err(|_| StateError::Unavailable)?;
    Ok(Reaped)
}

fn provider_cache_directory(path: &Path) -> bool {
    let components: Vec<_> = path.iter().filter_map(|p| p.to_str()).collect();
    components.len() >= 2
        && components.len() <= 6
        && components[..2] == [".terraform", "providers"]
        && (components.len() < 3 || components[2] == "registry.terraform.io")
        && components[2..].iter().all(|p| {
            p.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        })
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
