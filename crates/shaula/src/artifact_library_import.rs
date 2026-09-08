//! Trusted filesystem sources import once; database selections never follow files.

use super::{invalid, storage, DbArtifactPublisher, MAX_ARCHIVE};
use sha2::{Digest, Sha256};
use shaula_core::error::CoreResult;
use shaula_core::registry::TemplateSource;
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

    pub(super) async fn import_defaults(&self, root: &Path, now: i64) -> CoreResult<()> {
        let root = root.to_owned();
        let entries = tokio::task::spawn_blocking(move || default_directories(&root))
            .await
            .map_err(storage)??;
        for (key, directory) in entries {
            if self
                .store
                .template_source_exists(&key)
                .await
                .map_err(storage)?
            {
                continue;
            }
            let (bytes, manifest) = tokio::task::spawn_blocking(move || package(&directory))
                .await
                .map_err(storage)??;
            let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
            self.save(bytes, digest.clone(), now, true).await?;
            self.store
                .template_source_insert(
                    &TemplateSource {
                        key,
                        artifact_digest: digest,
                        platform: manifest.platform,
                        engine_ref: manifest.runtime.engine,
                    },
                    now,
                )
                .await
                .map_err(storage)?;
        }
        Ok(())
    }
}

fn default_directories(root: &Path) -> CoreResult<Vec<(String, PathBuf)>> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(storage(error)),
    };
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
