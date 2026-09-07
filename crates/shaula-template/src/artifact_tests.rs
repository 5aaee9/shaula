#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// Unit tests extracted to their own module to keep the host file
// within the 400-line limit (AGENTS.md).
use super::*;

fn tar_gz_fixture(files: &[(&str, &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, content.as_bytes())
            .unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
    encoder.finish().unwrap()
}

/// Builds a tar.gz from raw pre-serialized entries (for malformed
/// fixtures) and compresses it.
fn tar_gz_from_builder(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    build(&mut builder);
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
    encoder.finish().unwrap()
}

/// Writes one raw tar header with an arbitrary (possibly malicious)
/// path, bypassing builder validation.
fn append_raw_entry(builder: &mut tar::Builder<Vec<u8>>, name: &str, data: &[u8], entry_type: u8) {
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    let size_octal = format!("{:011o}", data.len());
    header[124..124 + size_octal.len()].copy_from_slice(size_octal.as_bytes());
    header[156] = entry_type;
    // Checksum: spaces while computing, then octal.
    for byte in &mut header[148..156] {
        *byte = b' ';
    }
    let sum: u32 = header.iter().map(|b| *b as u32).sum();
    let checksum = format!("{:06o}\0 ", sum);
    header[148..156].copy_from_slice(checksum.as_bytes());
    use std::io::Write;
    builder.get_mut().write_all(&header).unwrap();
    builder.get_mut().write_all(data).unwrap();
    // Pad to 512.
    let padding = (512 - data.len() % 512) % 512;
    builder.get_mut().write_all(&vec![0u8; padding]).unwrap();
}

fn digest_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[test]
fn executable_material_is_bound_to_the_archive_not_its_raw_digest() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tar_gz_fixture(&[
        ("profile.yaml", "manifest"),
        ("main.tf", "original"),
        (".terraform.lock.hcl", "lock"),
    ]);
    let digest = digest_of(&archive);
    let store = ArtifactStore::new(tmp.path().join("artifacts"));
    let published = store.publish(&archive, &digest).unwrap();
    let material =
        crate::artifact_integrity::material_digest(&published.final_path, &digest).unwrap();
    assert_ne!(material, digest);
    assert_eq!(
        material,
        crate::workspace::template_files_digest(&published.final_path).unwrap()
    );
    std::fs::write(published.final_path.join("main.tf"), "tampered").unwrap();
    assert!(crate::artifact_integrity::material_digest(&published.final_path, &digest).is_err());
    std::fs::write(published.final_path.join("main.tf"), "original").unwrap();
    std::fs::write(
        published.final_path.with_extension("tar.gz"),
        "tampered archive",
    )
    .unwrap();
    assert!(crate::artifact_integrity::material_digest(&published.final_path, &digest).is_err());
}

#[test]
fn publish_round_trip_and_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tar_gz_fixture(&[
        (
            "profile.yaml",
            "api_version: shaula.io/template-profile/v1\n",
        ),
        ("main.tf", "resource {}\n"),
    ]);
    let digest = digest_of(&archive);
    let store = ArtifactStore::new(tmp.path().join("artifacts"));

    let published = store.publish(&archive, &digest).unwrap();
    assert_eq!(published.digest, digest);
    assert!(published.manifest_yaml.contains("api_version"));

    // Idempotent republish.
    let again = store.publish(&archive, &digest).unwrap();
    assert_eq!(again.final_path, published.final_path);
}

#[test]
fn digest_mismatch_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tar_gz_fixture(&[("profile.yaml", "x")]);
    let store = ArtifactStore::new(tmp.path().join("artifacts"));
    let wrong = digest_of(b"other-bytes");
    let result = store.publish(&archive, &wrong);
    assert!(result.is_err());
    assert!(!tmp.path().join("artifacts").exists());
}

#[test]
fn traversal_entry_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tar_gz_from_builder(|builder| {
        append_raw_entry(builder, "../escape.txt", b"x", b'0');
        append_raw_entry(builder, "", b"", b'0'); // end-of-archive marker
    });

    let store = ArtifactStore::new(tmp.path().join("artifacts"));
    let result = store.publish(&archive, &digest_of(&archive));
    assert!(result.is_err(), "path traversal must be rejected");
    assert!(
        !tmp.path().join("escape.txt").exists(),
        "nothing escapes the staging dir"
    );
}

#[test]
fn symlink_entry_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tar_gz_from_builder(|builder| {
        append_raw_entry(builder, "evil-link", b"", b'2'); // '2' = symlink
        append_raw_entry(builder, "", b"", b'0');
    });

    let store = ArtifactStore::new(tmp.path().join("artifacts"));
    assert!(store.publish(&archive, &digest_of(&archive)).is_err());
}

#[test]
fn expansion_bomb_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let big = "y".repeat(1024);
    let archive = tar_gz_fixture(&[("big.txt", &big)]);
    let store = ArtifactStore::new(tmp.path().join("artifacts")).with_expansion_limit(16);
    assert!(store.publish(&archive, &digest_of(&archive)).is_err());
}

#[test]
fn missing_manifest_rejected_and_staging_cleaned() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tar_gz_fixture(&[("only.tf", "resource {}\n")]);
    let store = ArtifactStore::new(tmp.path().join("artifacts"));
    assert!(store.publish(&archive, &digest_of(&archive)).is_err());
    // No published or staging directories remain.
    let entries = std::fs::read_dir(tmp.path().join("artifacts"))
        .unwrap()
        .count();
    assert_eq!(entries, 0, "failed publication must not leave residue");
}

#[test]
fn read_bounded_enforces_limit() {
    assert!(read_bounded(&b"tiny"[..], 10).is_ok());
    assert!(read_bounded(&b"way-too-large"[..], 4).is_err());
}
