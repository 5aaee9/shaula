use super::*;
use crate::template::{TemplatePlatform, CONTAINER_BOOTSTRAP_CONTRACT, SETUP_INFO_CONTRACT};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn container_manifest() -> Result<ProfileManifest, Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/docker/profile.yaml");
    Ok(serde_yaml::from_str(&std::fs::read_to_string(path)?)?)
}

fn vm_manifest() -> Result<ProfileManifest, Box<dyn std::error::Error>> {
    let mut manifest = container_manifest()?;
    manifest.platform = "proxmox".into();
    manifest.bindings_contract = "shaula.bindings.proxmox/v1".into();
    manifest.container_bootstrap_contract = None;
    manifest.vm_image_contract = Some(PROXMOX_VM_IMAGE_CONTRACT.into());
    manifest.runner_image_digests.clear();
    Ok(manifest)
}

#[test]
fn operator_managed_vm_image_requires_explicit_admission() -> TestResult {
    let mut manifest = vm_manifest()?;
    manifest.validate_new_container_profile()?;
    assert_eq!(manifest.platform(), TemplatePlatform::Proxmox);
    assert_eq!(manifest.platform().metric_label(), "proxmox");
    let encoded = serde_yaml::to_string(&manifest)?;
    let decoded: ProfileManifest = serde_yaml::from_str(&encoded)?;
    assert_eq!(decoded, manifest);

    manifest.vm_image_contract = None;
    assert!(manifest.validate().is_err());
    Ok(())
}

#[test]
fn vm_contract_rejects_other_platforms_bindings_and_versions() -> TestResult {
    for platform in ["docker", "kubernetes", "virtual-machine"] {
        let mut manifest = vm_manifest()?;
        manifest.platform = platform.into();
        assert!(manifest.validate().is_err(), "accepted platform {platform}");
    }
    let mut manifest = vm_manifest()?;
    manifest.bindings_contract = "shaula.bindings.other/v1".into();
    assert!(manifest.validate().is_err());
    for contract in ["", "shaula.proxmox-template/v2"] {
        let mut manifest = vm_manifest()?;
        manifest.vm_image_contract = Some(contract.into());
        assert!(manifest.validate().is_err());
    }
    Ok(())
}

#[test]
fn vm_contract_rejects_claimed_pins_and_bootstrap_contracts() -> TestResult {
    let mut manifest = vm_manifest()?;
    manifest.runner_image_digests = container_manifest()?.runner_image_digests;
    assert!(manifest.validate().is_err());

    let mut manifest = vm_manifest()?;
    manifest.container_bootstrap_contract = Some(CONTAINER_BOOTSTRAP_CONTRACT.into());
    assert!(manifest.validate().is_err());

    let mut manifest = vm_manifest()?;
    manifest.input_contract_version = 2;
    manifest.setup_info_contract = Some(SETUP_INFO_CONTRACT.into());
    assert!(manifest.validate().is_err());
    Ok(())
}

#[test]
fn existing_manifests_keep_nonempty_immutable_image_authority() -> TestResult {
    let legacy = container_manifest()?;
    assert_eq!(legacy.vm_image_contract, None);
    assert!(!serde_yaml::to_string(&legacy)?.contains("vm_image_contract"));
    legacy.validate_new_container_profile()?;

    for images in [
        Vec::new(),
        vec!["registry.example/runner:latest".into()],
        vec!["registry.example/runner@sha256:1234".into()],
        vec![format!("registry.example/runner@sha256:{}", "z".repeat(64))],
        vec![legacy.runner_image_digests[0].clone(); 2],
    ] {
        let mut manifest = legacy.clone();
        manifest.runner_image_digests = images;
        assert!(manifest.validate().is_err());
        manifest.platform = "custom".into();
        manifest.container_bootstrap_contract = None;
        assert!(manifest.validate().is_err());
    }
    Ok(())
}
