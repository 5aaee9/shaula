//! Reconstruct the executable material commitment from the pinned archive.

use std::io::Write;
use std::path::Path;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

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

pub(crate) fn retain_archive(dir: &Path, bytes: &[u8], digest: &str) -> CoreResult<()> {
    let path = dir.with_extension("tar.gz");
    let parent = path.parent().ok_or_else(failure)?;
    std::fs::create_dir_all(parent).map_err(|_| failure())?;
    if path.exists() {
        let stored = crate::artifact::read_bounded(
            std::fs::File::open(&path).map_err(|_| failure())?,
            256 * 1024 * 1024,
        )?;
        crate::artifact::verify_digest(&stored, digest)?;
        return Ok(());
    }
    let mut staging = tempfile::NamedTempFile::new_in(parent).map_err(|_| failure())?;
    staging.write_all(bytes).map_err(|_| failure())?;
    staging.as_file().sync_all().map_err(|_| failure())?;
    match staging.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let stored = crate::artifact::read_bounded(
                std::fs::File::open(path).map_err(|_| failure())?,
                256 * 1024 * 1024,
            )?;
            crate::artifact::verify_digest(&stored, digest).map(|_| ())
        }
        Err(_) => Err(failure()),
    }
}

pub(crate) fn material_digest(dir: &Path, archive_digest: &str) -> CoreResult<String> {
    let source = std::fs::File::open(dir.with_extension("tar.gz")).map_err(|_| failure())?;
    let bytes = crate::artifact::read_bounded(source, 256 * 1024 * 1024)?;
    crate::artifact::verify_digest(&bytes, archive_digest)?;
    let scratch = tempfile::tempdir().map_err(|_| failure())?;
    crate::artifact::extract_tar_gz(&bytes, scratch.path(), 256 * 1024 * 1024)?;
    let expected = crate::workspace::template_files_digest(scratch.path())?;
    if crate::workspace::template_files_digest(dir)? != expected {
        return Err(failure());
    }
    Ok(expected)
}
