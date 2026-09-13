//! Repackage fixture variants so their manifest change has a real archive digest.
use sha2::{Digest, Sha256};
use shaula_core::template::ProfileManifest;
use std::io::Read;

pub fn with_bindings_schema(schema: &str) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    repackage(|path, bytes| {
        if path == std::path::Path::new("schemas/bindings.schema.json") {
            *bytes = schema.as_bytes().to_vec();
        }
    })
}

/// Drops one member from the fixture archive (for fail-closed coverage).
pub fn without_bindings_schema() -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    repackage(|path, bytes| {
        if path == std::path::Path::new("schemas/bindings.schema.json") {
            bytes.clear();
        }
    })
}

fn repackage(
    mut edit: impl FnMut(&std::path::Path, &mut Vec<u8>),
) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    let (_, original) = super::fixture_artifact();
    let decoder = flate2::read::GzDecoder::new(original.as_slice());
    let mut archive = tar::Archive::new(decoder);
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        edit(&path, &mut bytes);
        if bytes.is_empty() && path == std::path::Path::new("schemas/bindings.schema.json") {
            continue;
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, bytes.as_slice())?;
    }
    let bytes = builder.into_inner()?.finish()?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    Ok((digest, bytes))
}

pub fn with_manifest(
    edit: impl FnOnce(&mut ProfileManifest),
) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    let (_, original) = super::fixture_artifact();
    let decoder = flate2::read::GzDecoder::new(original.as_slice());
    let mut archive = tar::Archive::new(decoder);
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let mut edit = Some(edit);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if path == std::path::Path::new("profile.yaml") {
            let mut manifest: ProfileManifest = serde_yaml::from_slice(&bytes)?;
            edit.take().ok_or("duplicate manifest")?(&mut manifest);
            bytes = serde_yaml::to_string(&manifest)?.into_bytes();
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, bytes.as_slice())?;
    }
    if edit.is_some() {
        return Err("manifest missing".into());
    }
    let bytes = builder.into_inner()?.finish()?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    Ok((digest, bytes))
}
