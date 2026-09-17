//! Exercise bundled Tencent Cloud and Alibaba Cloud VM artifacts at publication boundaries.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use shaula_core::template::{
    TemplatePlatform, ALICLOUD_VM_IMAGE_CONTRACT, TENCENTCLOUD_VM_IMAGE_CONTRACT,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
struct CloudCase {
    name: &'static str,
    platform: TemplatePlatform,
    image_contract: &'static str,
    resource_type: &'static str,
    binding_count: usize,
    required_credentials: &'static [&'static str],
    defaults: &'static [(&'static str, &'static str)],
}

const CASES: [CloudCase; 2] = [
    CloudCase {
        name: "tencentcloud",
        platform: TemplatePlatform::TencentCloud,
        image_contract: TENCENTCLOUD_VM_IMAGE_CONTRACT,
        resource_type: "tencentcloud_instance",
        binding_count: 12,
        required_credentials: &["tencentcloud_secret_id", "tencentcloud_secret_key"],
        defaults: &[
            ("tencentcloud_instance_type", "\"S5.MEDIUM4\""),
            ("tencentcloud_system_disk_size", "50"),
            ("tencentcloud_internet_max_bandwidth_out", "0"),
            ("tencentcloud_cloud_init_cmd", "\"\""),
        ],
    },
    CloudCase {
        name: "alicloud",
        platform: TemplatePlatform::AliCloud,
        image_contract: ALICLOUD_VM_IMAGE_CONTRACT,
        resource_type: "alicloud_instance",
        binding_count: 10,
        required_credentials: &["alicloud_access_key_id", "alicloud_access_key_secret"],
        defaults: &[
            ("alicloud_instance_type", "\"ecs.g6.large\""),
            ("alicloud_system_disk_size", "40"),
            ("alicloud_internet_max_bandwidth_out", "0"),
            ("alicloud_cloud_init_cmd", "\"\""),
        ],
    },
];

fn source(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates")
        .join(name)
}

fn read(root: &Path, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::fs::read_to_string(root.join(name))?)
}

#[test]
fn bundled_public_cloud_manifests_admit_one_owned_instance() -> TestResult {
    for case in CASES {
        let root = source(case.name);
        crate::manifest::verify_artifact_shape(&root)?;
        let manifest = crate::manifest::parse_manifest(&read(&root, "profile.yaml")?)?;
        manifest.validate_new_container_profile()?;
        assert_eq!(manifest.platform(), case.platform, "{}", case.name);
        assert_eq!(
            manifest.vm_image_contract.as_deref(),
            Some(case.image_contract),
            "{}",
            case.name
        );
        assert!(manifest.runner_image_digests.is_empty(), "{}", case.name);
        assert_eq!(manifest.managed_resource_shape.len(), 1, "{}", case.name);
        let role = &manifest.managed_resource_shape[0];
        assert_eq!(role.role, "runner", "{}", case.name);
        assert_eq!(role.terraform_type, case.resource_type, "{}", case.name);
        assert_eq!(role.exact_count, 1, "{}", case.name);

        let policy = std::fs::read(root.join("runtime-policy.md"))?;
        assert_eq!(
            manifest.runtime_policy_digest,
            format!("sha256:{}", hex::encode(Sha256::digest(policy))),
            "{}",
            case.name
        );
    }
    Ok(())
}

#[test]
fn bundled_public_cloud_inputs_keep_platform_controls_out_of_fleets() -> TestResult {
    for case in CASES {
        let variables = crate::variables::discover_variables(&source(case.name), "artifact")?;
        assert!(variables.available, "{}: {:?}", case.name, variables.reason);
        assert!(variables.parameters.is_empty(), "{}", case.name);
        assert_eq!(
            variables.bindings.len(),
            case.binding_count,
            "{}",
            case.name
        );

        for &(key, expected) in case.defaults {
            let field = variables
                .bindings
                .iter()
                .find(|field| field.key == key)
                .ok_or_else(|| format!("{key} binding missing"))?;
            assert_eq!(field.default_value_json.as_deref(), Some(expected), "{key}");
            assert!(!field.required, "{key}");
            assert!(!field.sensitive, "{key}");
        }
        for &key in case.required_credentials {
            let field = variables
                .bindings
                .iter()
                .find(|field| field.key == key)
                .ok_or_else(|| format!("{key} credential binding missing"))?;
            assert!(field.required, "{key}");
            assert!(field.sensitive, "{key}");
            assert!(field.default_value_json.is_none(), "{key}");
        }
    }
    Ok(())
}

#[test]
fn bundled_public_cloud_bootstrap_keeps_provider_credentials_out_of_guest_material() -> TestResult {
    for case in CASES {
        let root = source(case.name);
        let guest_material = [
            read(&root, "user-data.tftpl")?,
            read(&root, "bootstrap.tftpl")?,
            read(&root, "runner-service.tftpl")?,
        ]
        .join("\n");
        for &credential in case.required_credentials {
            assert!(
                !guest_material.contains(credential),
                "{}: {credential}",
                case.name
            );
        }
        assert!(guest_material.contains("ACTIONS_RUNNER_INPUT_JITCONFIG"));
        assert!(guest_material.contains("RUNNER_SHA256"));
        assert!(guest_material.contains("ConditionPathExists=!/var/lib/shaula/started"));

        let main = read(&root, "main.tf")?;
        assert!(
            main.contains("system_disk_encrypted      = true")
                || main.contains("system_disk_encrypt        = true")
        );
        assert!(
            main.contains("deletion_protection           = false")
                || main.contains("disable_api_termination     = false")
        );

        let lock = read(&root, ".terraform.lock.hcl")?;
        assert!(
            lock.contains("h1:"),
            "{} lock lacks native checksum",
            case.name
        );
    }
    Ok(())
}
