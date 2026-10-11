//! Reconstruct the executable material commitment from the pinned archive.

use std::io::Write;
use std::path::Path;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

/// Upper bound (256 MiB) for reading a retained source archive and for the
/// expanded size of its extraction.
const ARCHIVE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;

fn failure() -> CoreError {
    CoreError::new(
        ReasonCode::TemplateInvalid,
        "artifact source archive unavailable or invalid",
    )
}

pub(crate) fn provenance_matches(
    original: &shaula_core::ports::PlanProvenance,
    archive: &str,
    material: &str,
) -> bool {
    if original.template_material_digest.is_empty() {
        // Legacy Create records stored the file-tree digest in artifact_digest.
        // The caller separately verifies the Generation's archive pin before using it.
        original.artifact_digest == material
    } else {
        original.artifact_digest == archive && original.template_material_digest == material
    }
}

/// Reads an already-retained archive (bounded) and verifies its digest.
fn verify_retained(path: &Path, digest: &str) -> CoreResult<()> {
    let stored = crate::artifact::read_bounded(
        std::fs::File::open(path).map_err(|_| failure())?,
        ARCHIVE_LIMIT_BYTES,
    )?;
    crate::artifact::verify_digest(&stored, digest).map(|_| ())
}

pub(crate) fn retain_archive(dir: &Path, bytes: &[u8], digest: &str) -> CoreResult<()> {
    let path = dir.with_extension("tar.gz");
    let parent = path.parent().ok_or_else(failure)?;
    std::fs::create_dir_all(parent).map_err(|_| failure())?;
    if path.exists() {
        return verify_retained(&path, digest);
    }
    let mut staging = tempfile::NamedTempFile::new_in(parent).map_err(|_| failure())?;
    staging.write_all(bytes).map_err(|_| failure())?;
    staging.as_file().sync_all().map_err(|_| failure())?;
    match staging.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            verify_retained(&path, digest)
        }
        Err(_) => Err(failure()),
    }
}

pub(crate) fn material_digest(dir: &Path, archive_digest: &str) -> CoreResult<String> {
    let source = std::fs::File::open(dir.with_extension("tar.gz")).map_err(|_| failure())?;
    let bytes = crate::artifact::read_bounded(source, ARCHIVE_LIMIT_BYTES)?;
    crate::artifact::verify_digest(&bytes, archive_digest)?;
    let scratch = tempfile::tempdir().map_err(|_| failure())?;
    crate::artifact::extract_tar_gz(&bytes, scratch.path(), ARCHIVE_LIMIT_BYTES)?;
    let expected = crate::workspace::template_files_digest(scratch.path())?;
    if crate::workspace::template_files_digest(dir)? != expected {
        return Err(failure());
    }
    Ok(expected)
}

/// [`material_digest`] on the blocking pool (gunzip + untar + tree walk).
pub(crate) async fn material_digest_off_thread(
    dir: &Path,
    archive_digest: &str,
) -> CoreResult<String> {
    let dir = dir.to_path_buf();
    let archive_digest = archive_digest.to_string();
    crate::engine::off_thread(move || material_digest(&dir, &archive_digest)).await
}
