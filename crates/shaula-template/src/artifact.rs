//! Content-addressed artifact store: safe extraction, digest verification
//! and atomic publication. Publication is the high-trust remote-code
//! admission surface, so every hazard fails closed.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::EntryType;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

pub const SUPPORTED_MEDIA: &str = "application/gzip";
const DEFAULT_MAX_EXPANSION_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, msg)
}

/// Result of an accepted publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedArtifact {
    pub digest: String,
    pub size_bytes: u64,
    pub final_path: PathBuf,
    pub manifest_yaml: String,
}

/// Verifies the raw bytes against the declared digest and returns the
/// computed hex digest.
pub fn verify_digest(bytes: &[u8], declared: &str) -> CoreResult<String> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = format!("sha256:{}", hex::encode(hasher.finalize()));
    if digest != declared {
        return Err(err("declared digest does not match content"));
    }
    Ok(digest)
}

/// Validates one entry path inside an archive: relative, contained, no
/// traversal, no absolute/UNC/device forms, no duplicate normalization.
fn sanitize_entry_path(raw: &Path, seen: &mut Vec<String>) -> CoreResult<PathBuf> {
    let text = raw
        .to_str()
        .ok_or_else(|| err("archive entry path is not valid UTF-8"))?;
    if text.is_empty() || text.len() > 512 {
        return Err(err("archive entry path out of bounds"));
    }
    let path = Path::new(text);
    if path.is_absolute() {
        return Err(err("absolute path inside archive"));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return Err(err("traversal or reserved component inside archive")),
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(err("empty archive entry path"));
    }
    let normalized = clean.to_string_lossy().replace('\\', "/").to_lowercase();
    if seen.contains(&normalized) {
        return Err(err("duplicate normalized path inside archive"));
    }
    seen.push(normalized);
    Ok(clean)
}

/// Extracts a gzipped tarball into `dest` under the safety policy: regular
/// files and directories only, expansion bounded, no links or devices.
pub fn extract_tar_gz(bytes: &[u8], dest: &Path, max_expansion: u64) -> CoreResult<()> {
    let decoder = GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(false);
    archive.set_unpack_xattrs(false);
    archive.set_overwrite(true);

    let mut seen: Vec<String> = Vec::new();
    let mut total: u64 = 0;
    let entries = archive
        .entries()
        .map_err(|e| err(format!("archive unreadable: {e}")))?;
    let mut count = 0usize;
    for entry in entries {
        let mut entry = entry.map_err(|e| err(format!("archive entry unreadable: {e}")))?;
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(err("archive contains too many entries"));
        }
        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::Regular | EntryType::Directory => {}
            _ => return Err(err("link, device or special entry inside archive")),
        }
        let entry_path = entry
            .path()
            .map_err(|e| err(format!("archive entry path invalid: {e}")))?;
        let relative = sanitize_entry_path(entry_path.as_ref(), &mut seen)?;
        let target = dest.join(relative);
        if entry_type == EntryType::Regular {
            let size = entry.size();
            total += size;
            if total > max_expansion {
                return Err(err("archive expansion exceeds limit"));
            }
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| err(format!("cannot create extraction dir: {e}")))?;
        }
        if entry_type == EntryType::Regular {
            let mut file = std::fs::File::create(&target)
                .map_err(|e| err(format!("cannot create extracted file: {e}")))?;
            std::io::copy(&mut entry, &mut file)
                .map_err(|e| err(format!("cannot write extracted file: {e}")))?;
        } else {
            std::fs::create_dir_all(&target)
                .map_err(|e| err(format!("cannot create extracted dir: {e}")))?;
        }
    }
    Ok(())
}

/// The artifact store rooted at a configured directory.
pub struct ArtifactStore {
    root: PathBuf,
    max_expansion: u64,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_expansion: DEFAULT_MAX_EXPANSION_BYTES,
        }
    }

    pub fn with_expansion_limit(mut self, limit: u64) -> Self {
        self.max_expansion = limit;
        self
    }

    /// Publishes bytes under their digest: verify → extract safely into a
    /// staging dir → fsync-less atomic rename into the content-addressed
    /// path. Idempotent for an already-published digest.
    pub fn publish(&self, bytes: &[u8], declared_digest: &str) -> CoreResult<PublishedArtifact> {
        let digest = verify_digest(bytes, declared_digest)?;
        // The digest was just verified against the bytes, so the layout
        // authority always resolves it.
        let final_path = self
            .path_for(&digest)
            .ok_or_else(|| err("artifact digest is malformed"))?;
        if final_path.exists() {
            crate::artifact_integrity::retain_archive(&final_path, bytes, &digest)?;
            let manifest = std::fs::read_to_string(final_path.join("profile.yaml"))
                .map_err(|e| err(format!("published artifact unreadable: {e}")))?;
            return Ok(PublishedArtifact {
                digest,
                size_bytes: bytes.len() as u64,
                final_path,
                manifest_yaml: manifest,
            });
        }

        let staging = {
            // Unique per call so concurrent publications of the same digest
            // never share a staging directory.
            let unique = format!(
                "staging-{}-{}",
                hex::encode(
                    Sha256::digest(declared_digest.as_bytes())
                        .get(..8)
                        .map_or(&[][..], |s| s)
                ),
                shaula_core::auth::new_attempt_id()
            );
            self.root.join(unique)
        };
        std::fs::create_dir_all(&staging)
            .map_err(|e| err(format!("cannot create staging dir: {e}")))?;

        let result = (|| -> CoreResult<String> {
            extract_tar_gz(bytes, &staging, self.max_expansion)?;
            let manifest_path = staging.join("profile.yaml");
            if !manifest_path.is_file() {
                return Err(err("artifact missing profile.yaml manifest"));
            }
            let manifest = std::fs::read_to_string(&manifest_path)
                .map_err(|e| err(format!("manifest unreadable: {e}")))?;
            Ok(manifest)
        })();

        match result {
            Ok(manifest) => {
                crate::artifact_integrity::retain_archive(&final_path, bytes, &digest)?;
                if let Some(parent) = final_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| err(format!("cannot create artifact root: {e}")))?;
                }
                std::fs::rename(&staging, &final_path)
                    .map_err(|e| err(format!("atomic publication failed: {e}")))?;
                Ok(PublishedArtifact {
                    digest,
                    size_bytes: bytes.len() as u64,
                    final_path,
                    manifest_yaml: manifest,
                })
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }

    /// Resolves one artifact directory through the single canonical layout
    /// authority. `None` for a malformed digest: never guess a path.
    pub fn path_for(&self, digest: &str) -> Option<PathBuf> {
        shaula_core::artifact_layout::artifact_dir(&self.root, digest)
    }

    /// Reads the manifest of a published artifact by digest.
    pub fn manifest(&self, digest: &str) -> CoreResult<String> {
        let path = self
            .path_for(digest)
            .ok_or_else(|| err("artifact digest is malformed"))?
            .join("profile.yaml");
        std::fs::read_to_string(path).map_err(|_| err("artifact is not published"))
    }
}

/// Reads a bounded chunk from a reader; used by HTTP upload before
/// publication.
pub fn read_bounded<R: Read>(mut reader: R, limit: u64) -> CoreResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|e| err(format!("read failed: {e}")))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > limit {
            return Err(err("upload exceeds body limit"));
        }
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
