//! Exercise bundled AWS material through the same manifest/variable admission as publication.

use std::path::PathBuf;

use sha2::{Digest, Sha256};
use shaula_core::template::{TemplatePlatform, AWS_VM_IMAGE_CONTRACT};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/aws")
}

#[test]
fn bundled_aws_manifest_admits_canonical_image_and_one_owned_instance() -> TestResult {
    let root = source();
    crate::manifest::verify_artifact_shape(&root)?;
    let manifest =
        crate::manifest::parse_manifest(&std::fs::read_to_string(root.join("profile.yaml"))?)?;
    manifest.validate_new_container_profile()?;
    assert_eq!(manifest.platform(), TemplatePlatform::Aws);
    assert_eq!(
        manifest.vm_image_contract.as_deref(),
        Some(AWS_VM_IMAGE_CONTRACT)
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
    assert_eq!(roles, vec![("runner", "aws_instance", 1)]);
    let policy = std::fs::read(root.join("runtime-policy.md"))?;
    assert_eq!(
        manifest.runtime_policy_digest,
        format!("sha256:{}", hex::encode(Sha256::digest(policy)))
    );
    Ok(())
}

#[test]
fn bundled_aws_inputs_discover_defaults_without_exposing_platform_controls_to_fleets() -> TestResult
{
    let variables = crate::variables::discover_variables(&source(), "artifact")?;
    assert!(variables.available);
    assert!(variables.parameters.is_empty());
    assert_eq!(variables.bindings.len(), 9);
    for (key, expected) in [
        (
            "aws_ami_name",
            "\"ubuntu/images/hvm-ssd/ubuntu-jammy-22.04-amd64-server-*\"",
        ),
        ("aws_instance_type", "\"t3.large\""),
        ("aws_subnet_id", "\"\""),
        ("aws_root_volume_gb", "30"),
        ("aws_cloud_init_cmd", "\"\""),
    ] {
        let field = variables
            .bindings
            .iter()
            .find(|field| field.key == key)
            .ok_or("binding missing")?;
        assert_eq!(field.default_value_json.as_deref(), Some(expected), "{key}");
        assert!(!field.sensitive, "{key}");
    }
    for key in ["aws_access_key_id", "aws_secret_access_key"] {
        let field = variables
            .bindings
            .iter()
            .find(|field| field.key == key)
            .ok_or("credential binding missing")?;
        assert!(field.required && field.sensitive, "{key}");
        assert!(field.default_value_json.is_none(), "{key}");
    }
    let region = variables
        .bindings
        .iter()
        .find(|field| field.key == "aws_region")
        .ok_or("region missing")?;
    assert!(region.required);
    assert!(region.default_value_json.is_none());
    Ok(())
}
