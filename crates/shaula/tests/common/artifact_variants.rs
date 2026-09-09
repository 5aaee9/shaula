//! Repackage fixture variants so their manifest change has a real archive digest.
use sha2::{Digest, Sha256};
use shaula_core::template::ProfileManifest;
use std::io::Read;

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
