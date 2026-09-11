#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Bundled Profile smoke tests: both shipped templates must pass the full
//! publication pipeline (package → digest → artifact store → manifest
//! validation → managed shape contract).

use shaula_template::artifact::ArtifactStore;
use shaula_template::manifest::{parse_manifest, verify_artifact_shape};

fn package_template(dir: &std::path::Path) -> (String, Vec<u8>) {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    fn walk(dir: &std::path::Path, base: &std::path::Path, files: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                walk(&path, base, files);
            } else {
                files.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    walk(dir, dir, &mut files);

    let mut builder = tar::Builder::new(Vec::new());
    for (name, content) in &files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, content.as_slice())
            .unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
    let bytes = encoder.finish().unwrap();
    let digest = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };
    (digest, bytes)
}

fn smoke_test_template(name: &str, expected_platform: &str) {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let template_dir = workspace_root.join("templates").join(name);
    if !template_dir.exists() {
        panic!("bundled template {name} missing from the repository");
    }

    let (digest, bytes) = package_template(&template_dir);
    let tmp = tempfile::tempdir().unwrap();
    let store = ArtifactStore::new(tmp.path().join("artifacts"));
    let published = store
        .publish(&bytes, &digest)
        .expect("publication must succeed");

    // Manifest is the sole authority for platform identity.
    let manifest = parse_manifest(&published.manifest_yaml).expect("manifest must validate");
    assert_eq!(manifest.platform, expected_platform);

    // Required shape files exist in the extracted artifact.
    verify_artifact_shape(&published.final_path).expect("artifact shape must pass");

    // The bindings digest binds this exact revision, keyed not plaintext.
    let digest_a = shaula_core::template::BindingsDigest::from_keyed_material(
        &format!("{name}/1"),
        b"server-key",
    )
    .unwrap();
    let digest_b = shaula_core::template::BindingsDigest::from_keyed_material(
        &format!("{name}/2"),
        b"server-key",
    )
    .unwrap();
    assert_ne!(digest_a, digest_b);
}

#[test]
fn kubernetes_template_publishes_and_validates() {
    smoke_test_template("kubernetes", "kubernetes");
}

#[test]
fn docker_template_publishes_and_validates() {
    smoke_test_template("docker", "docker");
}

#[test]
fn proxmox_template_publishes_and_validates() {
    smoke_test_template("proxmox", "proxmox");
}

#[test]
fn aws_template_publishes_and_validates() {
    smoke_test_template("aws", "aws");
}

#[test]
fn bundled_templates_declare_distinct_contracts() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |name: &str| {
        std::fs::read_to_string(
            workspace_root
                .join("templates")
                .join(name)
                .join("profile.yaml"),
        )
        .unwrap()
    };
    let k8s = read("kubernetes");
    let docker = read("docker");
    let proxmox = read("proxmox");
    let aws = read("aws");
    assert!(k8s.contains("shaula.bindings.kubernetes/v1"));
    assert!(docker.contains("shaula.bindings.docker/v1"));
    assert!(proxmox.contains("shaula.bindings.proxmox/v1"));
    assert!(aws.contains("shaula.bindings.aws/v1"));
    assert!(k8s.contains("kubernetes_secret_v1") && k8s.contains("kubernetes_pod_v1"));
    assert!(docker.contains("docker_container"));
    assert!(proxmox.contains("proxmox_qemu_vm") && proxmox.contains("proxmox_nocloud_iso"));
    assert!(aws.contains("aws_instance"));
    // VM platforms use the explicit operator-managed image contract; the
    // container platforms keep immutable OCI digest pins instead.
    assert!(proxmox.contains("vm_image_contract: shaula.proxmox-template/v1"));
    assert!(aws.contains("vm_image_contract: shaula.aws-ami/v1"));
    assert!(!k8s.contains("vm_image_contract"));
    assert!(!docker.contains("vm_image_contract"));
}
