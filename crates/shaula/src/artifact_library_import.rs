//! Trusted filesystem sources form the current default catalog; revisions stay pinned.

use super::{invalid, storage, DbArtifactPublisher, MAX_ARCHIVE};
use sha2::{Digest, Sha256};
use shaula_core::error::CoreResult;
use shaula_core::registry::TemplateSource;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

impl DbArtifactPublisher {
    pub(super) async fn import_legacy(&self, now: i64) -> CoreResult<()> {
        let root = self.root.clone();
        let paths = tokio::task::spawn_blocking(move || legacy_archives(&root))
            .await
            .map_err(storage)??;
        for (digest, path) in paths {
            if self
                .store
                .artifact_archive_get(&digest)
                .await
                .map_err(storage)?
                .is_some()
            {
                continue;
            }
            let bytes = tokio::task::spawn_blocking(move || {
                shaula_template::artifact::read_bounded(
                    std::fs::File::open(path).map_err(storage)?,
                    MAX_ARCHIVE,
                )
            })
            .await
            .map_err(storage)??;
            self.save(bytes, digest, now, false).await?;
        }
        Ok(())
    }

    pub(super) async fn sync_defaults(&self, roots: &[PathBuf], now: i64) -> CoreResult<()> {
        let mut entries = BTreeMap::new();
        for root in roots {
            let root = root.clone();
            for (key, directory) in tokio::task::spawn_blocking(move || default_directories(&root))
                .await
                .map_err(storage)??
            {
                if entries.insert(key, directory).is_some() {
                    return Err(invalid(
                        "duplicate template source key in configured directories",
                    ));
                }
            }
        }
        let mut sources = Vec::with_capacity(entries.len());
        for (key, directory) in entries {
            let (bytes, manifest) = tokio::task::spawn_blocking(move || package(&directory))
                .await
                .map_err(storage)??;
            let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
            self.save(bytes, digest.clone(), now, true).await?;
            sources.push(TemplateSource {
                key,
                artifact_digest: digest,
                platform: manifest.platform,
                engine_ref: manifest.runtime.engine,
            });
        }
        // A failed validation/cache write leaves the previous catalog intact.
        // Original archives are immutable and may be reused on startup retry.
        self.store
            .template_sources_replace(&sources, now)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

fn default_directories(root: &Path) -> CoreResult<Vec<(String, PathBuf)>> {
    let metadata = std::fs::symlink_metadata(root).map_err(storage)?;
    if redirected(&metadata) {
        return Err(invalid("template source root must not be redirected"));
    }
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root).map_err(storage)? {
        let entry = entry.map_err(storage)?;
        if redirected(&std::fs::symlink_metadata(entry.path()).map_err(storage)?) {
            return Err(invalid(
                "template sources cannot contain redirected directories",
            ));
        }
        if !entry.file_type().map_err(storage)?.is_dir() {
            continue;
        }
        let key = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("template source key is not UTF-8"))?;
        if key.starts_with('.') {
            continue;
        }
        shaula_core::auth::validate_profile_key_for_fleet_ref(&key)?;
        result.push((key, entry.path()));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

fn legacy_archives(root: &Path) -> CoreResult<Vec<(String, PathBuf)>> {
    let mut result = Vec::new();
    for prefix in std::fs::read_dir(root).map_err(storage)? {
        let prefix = prefix.map_err(storage)?;
        let name = prefix.file_name().to_string_lossy().into_owned();
        if name.len() != 2 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if !prefix.file_type().map_err(storage)?.is_dir() {
            return Err(storage("artifact prefix is not a directory"));
        }
        for entry in std::fs::read_dir(prefix.path()).map_err(storage)? {
            let entry = entry.map_err(storage)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(hex) = name.strip_suffix(".tar.gz") else {
                continue;
            };
            let digest = format!("sha256:{hex}");
            let expected = shaula_core::artifact_layout::artifact_dir(root, &digest)
                .ok_or_else(|| invalid("legacy artifact filename is malformed"))?
                .with_extension("tar.gz");
            if expected != entry.path() || !entry.file_type().map_err(storage)?.is_file() {
                return Err(invalid("legacy artifact path is not canonical"));
            }
            result.push((digest, entry.path()));
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

pub(super) fn package(
    directory: &Path,
) -> CoreResult<(Vec<u8>, shaula_core::template::ProfileManifest)> {
    let manifest = std::fs::read_to_string(directory.join("profile.yaml")).map_err(storage)?;
    let manifest = shaula_template::manifest::parse_manifest(&manifest)?;
    let mut files = Vec::new();
    collect_files(directory, directory, &mut files)?;
    files.sort();
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut total = 0u64;
    for relative in files {
        let bytes = shaula_template::artifact::read_bounded(
            std::fs::File::open(directory.join(&relative)).map_err(storage)?,
            MAX_ARCHIVE,
        )?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("template source exceeds size limit"))?;
        if total > MAX_ARCHIVE {
            return Err(invalid("template source exceeds size limit"));
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder
            .append_data(&mut header, relative, bytes.as_slice())
            .map_err(storage)?;
    }
    let bytes = builder
        .into_inner()
        .map_err(storage)?
        .finish()
        .map_err(storage)?;
    Ok((bytes, manifest))
}

fn collect_files(directory: &Path, root: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
    if directory
        .strip_prefix(root)
        .map_err(storage)?
        .components()
        .count()
        > 16
    {
        return Err(invalid("template source directory exceeds depth limit"));
    }
    for entry in std::fs::read_dir(directory).map_err(storage)? {
        let entry = entry.map_err(storage)?;
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(storage)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let top = directory == root;
        let ty = entry.file_type().map_err(storage)?;
        if !top
            && relative != Path::new("schemas/bindings.schema.json")
            && relative != Path::new("schemas/parameters.schema.json")
        {
            continue;
        }
        if top
            && name != "schemas"
            && !(name.ends_with(".tf")
                || name.ends_with(".tf.json")
                || matches!(
                    name.as_str(),
                    "profile.yaml" | ".terraform.lock.hcl" | "runtime-policy.md"
                ))
        {
            continue;
        }
        if redirected(&std::fs::symlink_metadata(&path).map_err(storage)?) {
            return Err(invalid("template sources cannot contain symlinks"));
        }
        if ty.is_dir() {
            collect_files(&path, root, files)?;
        } else if ty.is_file() {
            files.push(relative.to_owned());
            if files.len() > 10_000 {
                return Err(invalid("template source contains too many files"));
            }
        } else {
            return Err(invalid("template sources must contain regular files"));
        }
    }
    Ok(())
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
