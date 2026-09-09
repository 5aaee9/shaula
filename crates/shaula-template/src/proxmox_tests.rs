//! Exercise bundled VM material through the same manifest/variable admission as publication.

use std::path::PathBuf;

use sha2::{Digest, Sha256};
use shaula_core::template::{TemplatePlatform, PROXMOX_VM_IMAGE_CONTRACT};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/proxmox")
}

#[test]
fn bundled_proxmox_manifest_admits_operator_image_and_two_owned_resources() -> TestResult {
    let root = source();
    crate::manifest::verify_artifact_shape(&root)?;
    let manifest =
        crate::manifest::parse_manifest(&std::fs::read_to_string(root.join("profile.yaml"))?)?;
    manifest.validate_new_container_profile()?;
    assert_eq!(manifest.platform(), TemplatePlatform::Proxmox);
    assert_eq!(
        manifest.vm_image_contract.as_deref(),
        Some(PROXMOX_VM_IMAGE_CONTRACT)
    );
    assert!(manifest.runner_image_digests.is_empty());
    let roles: Vec<_> = manifest
        .managed_resource_shape
        .iter()
        .map(|role| {
            (
                role.role.as_str(),
                role.terraform_type.as_str(),
                role.exact_count,
            )
        })
        .collect();
    assert_eq!(
        roles,
        vec![
            ("bootstrap", "proxmox_nocloud_iso", 1),
            ("runner", "proxmox_qemu_vm", 1)
        ]
    );
    let policy = std::fs::read(root.join("runtime-policy.md"))?;
    assert_eq!(
        manifest.runtime_policy_digest,
        format!("sha256:{}", hex::encode(Sha256::digest(policy)))
    );
    Ok(())
}

#[test]
fn bundled_proxmox_inputs_discover_defaults_without_exposing_platform_controls_to_fleets(
) -> TestResult {
    let variables = crate::variables::discover_variables(&source(), "artifact")?;
    assert!(variables.available);
    assert!(variables.parameters.is_empty());
    assert_eq!(variables.bindings.len(), 7);
    for (key, expected) in [
        ("proxmox_insecure", "true"),
        ("proxmox_template_name", "\"GitHub-Runner\""),
        ("proxmox_vmid_begin", "100"),
        ("proxmox_iso_storage", "\"local\""),
        ("proxmox_cloud_init_cmd", "\"\""),
    ] {
        let field = variables
            .bindings
            .iter()
            .find(|field| field.key == key)
            .ok_or("binding missing")?;
        assert_eq!(field.default_value_json.as_deref(), Some(expected), "{key}");
        assert!(!field.sensitive, "{key}");
    }
    let token = variables
        .bindings
        .iter()
        .find(|field| field.key == "proxmox_token")
        .ok_or("token missing")?;
    assert!(token.required && token.sensitive);
    assert!(token.default_value_json.is_none());
    let host = variables
        .bindings
        .iter()
        .find(|field| field.key == "proxmox_host")
        .ok_or("host missing")?;
    assert!(host.required);
    assert!(host.default_value_json.is_none());
    Ok(())
}

#[test]
fn real_proxmox_saved_plans_pass_the_runtime_admission_boundary() -> TestResult {
    use shaula_core::plan::{admit_create_plan, admit_destroy_plan_json, parse_plan};

    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proxmox");
    let manifest =
        crate::manifest::parse_manifest(&std::fs::read_to_string(source().join("profile.yaml"))?)?;
    let create = serde_json::from_str(&std::fs::read_to_string(fixtures.join("create.json"))?)?;
    let destroy = serde_json::from_str(&std::fs::read_to_string(fixtures.join("destroy.json"))?)?;
    let create = parse_plan(&create)?;
    admit_create_plan(&create, true, &manifest.managed_resource_shape)?;
    let owned = create
        .instances
        .iter()
        .filter(|resource| resource.mode == "managed")
        .map(|resource| resource.address.clone())
        .collect::<Vec<_>>();
    assert_eq!(owned.len(), 2);
    admit_destroy_plan_json(&destroy, &owned, &manifest.managed_resource_shape)?;

    // Terraform skips the owned ISO's precondition on Destroy. It must remain
    // unacceptable as a Create plan, rather than weakening general check policy.
    assert!(parse_plan(&destroy).is_err());
    Ok(())
}
