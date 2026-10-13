use super::*;
use crate::fleet::FleetProviderKind::{Forgejo, Github, Gitlab};
use crate::ports::gitlab::GitlabBootstrapMaterial;
use crate::template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope};
use crate::template::{GitlabVmBootstrap, GITLAB_VM_BOOTSTRAP_CONTRACT};
use serde_json::{json, Map};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn manifest(platform: &str) -> TestResult<ProfileManifest> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates")
        .join(platform)
        .join("profile.yaml");
    Ok(serde_yaml::from_str(&std::fs::read_to_string(path)?)?)
}

fn input() -> TestResult<ShaulaInputEnvelope> {
    let material = ForgejoBootstrapMaterial::new(
        "https://forgejo.test",
        "runner-uuid",
        SecretString::new("vm-token-canary"),
        vec!["linux:host".into()],
    )?;
    let mut input = ShaulaInputEnvelope::new(
        GenerationIdentity {
            fleet_key: "fleet".into(),
            scale_set_id: None,
            id: "gen".into(),
            runner_name: "runner".into(),
            generation_name: "generation".into(),
        },
        String::new(),
        BindingsDigest("digest".into()),
    );
    input
        .bindings
        .insert("runner_backend".into(), json!("forgejo"));
    input.forgejo = Some(material.identity());
    input.forgejo_vm = Some(ForgejoVmBootstrap::from_registration(&material));
    Ok(input)
}

fn gitlab_material() -> TestResult<GitlabBootstrapMaterial> {
    Ok(GitlabBootstrapMaterial::new(
        "https://gitlab.test/sub",
        42,
        "runner $(name)",
        SecretString::new("glrt-vm-token-canary"),
    )?)
}

fn gitlab_input() -> TestResult<ShaulaInputEnvelope> {
    let material = gitlab_material()?;
    let mut input = input()?;
    input
        .bindings
        .insert("runner_backend".into(), json!("gitlab"));
    input.forgejo = None;
    input.forgejo_vm = None;
    input.gitlab = Some(material.identity());
    input.gitlab_vm = Some(GitlabVmBootstrap::from_registration(&material));
    Ok(input)
}

#[test]
fn vm_backends_require_explicit_cloud_init_credential_contract() -> TestResult {
    for platform in ["proxmox", "aws", "tencentcloud", "alicloud"] {
        let mut manifest = manifest(platform)?;
        let bindings = serde_json::from_value(json!({"runner_backend":"forgejo"}))?;
        manifest.validate_bindings_for_provider(&Map::new(), Github)?;
        manifest.validate_bindings_for_provider(&bindings, Forgejo)?;
        assert!(manifest
            .validate_bindings_for_provider(&bindings, Github)
            .is_err());
        assert_eq!(
            manifest.forgejo_vm_bootstrap_contract.as_deref(),
            Some(FORGEJO_VM_BOOTSTRAP_CONTRACT)
        );
        manifest.validate_forgejo_targets(&["linux:host".into()])?;
        assert!(manifest
            .validate_forgejo_targets(&["linux:docker://ubuntu".into()])
            .is_err());
        // GitLab VMs are admitted only through their own cloud-init contract.
        let gitlab = serde_json::from_value(json!({"runner_backend":"gitlab"}))?;
        manifest.validate_bindings_for_provider(&gitlab, Gitlab)?;
        manifest.validate_gitlab_generation_target()?;
        assert!(manifest.validate_gitlab_target().is_err());
        assert_eq!(
            manifest.gitlab_vm_bootstrap_contract.as_deref(),
            Some(GITLAB_VM_BOOTSTRAP_CONTRACT)
        );
        let mut without_gitlab_contract = manifest.clone();
        without_gitlab_contract.gitlab_vm_bootstrap_contract = None;
        assert!(without_gitlab_contract.validate().is_err());
        manifest.forgejo_vm_bootstrap_contract = None;
        assert!(manifest.validate().is_err());
        // Old VM artifacts still work as GitHub-only and cannot receive the token.
        manifest.runner_backends.clear();
        manifest.gitlab_vm_bootstrap_contract = None;
        manifest.validate_bindings_for_provider(&Map::new(), Github)?;
        assert!(manifest.validate_for_provider(Forgejo).is_err());
    }
    Ok(())
}

#[test]
fn vm_contract_cannot_authorize_other_platforms_or_bootstrap_channels() -> TestResult {
    let original = manifest("aws")?;
    for platform in ["docker", "kubernetes", "unknown"] {
        let mut candidate = original.clone();
        candidate.platform = platform.into();
        assert!(candidate.validate().is_err());
    }
    for contract in ["", "shaula.forgejo-vm-cloud-init/v2"] {
        let mut candidate = original.clone();
        candidate.forgejo_vm_bootstrap_contract = Some(contract.into());
        assert!(candidate.validate().is_err());
    }
    for contract in [
        "",
        "shaula.gitlab-vm-cloud-init/v2",
        FORGEJO_VM_BOOTSTRAP_CONTRACT,
    ] {
        let mut candidate = original.clone();
        candidate.gitlab_vm_bootstrap_contract = Some(contract.into());
        assert!(candidate.validate().is_err());
    }
    let mut candidate = original.clone();
    candidate.container_bootstrap_contract =
        Some(crate::template::CONTAINER_BOOTSTRAP_CONTRACT.into());
    assert!(candidate.validate().is_err());
    let mut candidate = original.clone();
    candidate.vm_image_contract = None;
    assert!(candidate.validate().is_err());
    let mut candidate = original;
    candidate.runner_backends.clear();
    assert!(candidate.validate().is_err());
    Ok(())
}

#[test]
fn vm_token_is_required_only_for_the_selected_forgejo_vm() -> TestResult {
    for platform in ["proxmox", "aws", "tencentcloud", "alicloud"] {
        let manifest = manifest(platform)?;
        let mut input = input()?;
        input.validate_for_manifest(&manifest)?;
        let token = input.forgejo_vm.take();
        assert!(input.validate_for_manifest(&manifest).is_err());
        input.forgejo_vm = token;
        input.bindings.clear();
        assert!(input.validate_for_manifest(&manifest).is_err());
        input.forgejo = None;
        assert!(input.validate_for_manifest(&manifest).is_err());
        input.forgejo_vm = None;
        input.jit_config = "jit".into();
        input.generation.scale_set_id = Some(1);
        input.validate_for_manifest(&manifest)?;
        assert!(serde_json::to_value(&input)?.get("forgejo_vm").is_none());

        // GitLab: the config.toml must belong to exactly the named registration.
        let mut candidate = gitlab_input()?;
        candidate.validate_for_manifest(&manifest)?;
        let config = candidate.gitlab_vm.take();
        assert!(candidate.validate_for_manifest(&manifest).is_err());
        candidate.gitlab_vm = config;
        if let Some(identity) = candidate.gitlab.as_mut() {
            identity.runner_id = 43;
        }
        assert!(candidate.validate().is_err());
        let mut candidate = gitlab_input()?;
        candidate.forgejo = self::input()?.forgejo;
        assert!(candidate.validate().is_err());
    }
    for platform in ["docker", "kubernetes"] {
        assert!(input()?
            .validate_for_manifest(&manifest(platform)?)
            .is_err());
        assert!(gitlab_input()?
            .validate_for_manifest(&manifest(platform)?)
            .is_err());
    }
    Ok(())
}

#[test]
fn vm_token_serializes_only_as_protected_input_and_debug_redacts_it() -> TestResult {
    let input = input()?;
    assert!(!format!("{input:?}").contains("vm-token-canary"));
    assert!(!format!("{:?}", input.forgejo_vm).contains("vm-token-canary"));
    let wire = input.to_tfvars()?;
    let json: serde_json::Value = serde_json::from_str(&wire)?;
    assert_eq!(
        json["shaula"]["forgejo_vm"],
        json!({"token":"vm-token-canary"})
    );
    assert_eq!(
        serde_json::from_value::<ShaulaInputEnvelope>(json["shaula"].clone())?,
        input
    );
    for token in [
        json!(""),
        json!("bad\nvalue"),
        json!("x".repeat(4097)),
        json!(false),
    ] {
        assert!(serde_json::from_value::<ForgejoVmBootstrap>(json!({"token":token})).is_err());
    }
    assert!(serde_json::from_value::<ForgejoVmBootstrap>(
        json!({"token":"ok", "admin_token":"no"})
    )
    .is_err());

    // GitLab: one canonical shell-executor config.toml, never another shape.
    let input = gitlab_input()?;
    let config = gitlab_material()?.config_toml();
    assert!(!format!("{input:?}").contains("vm-token-canary"));
    assert!(!format!("{:?}", input.gitlab_vm).contains("vm-token-canary"));
    let json: serde_json::Value = serde_json::from_str(&input.to_tfvars()?)?;
    assert_eq!(
        json["shaula"]["gitlab_vm"],
        json!({ "config_toml": config })
    );
    assert_eq!(
        serde_json::from_value::<ShaulaInputEnvelope>(json["shaula"].clone())?,
        input
    );
    for invalid in [
        config.replace("\"shell\"", "\"docker\""),
        config.replace("glrt-", "glpat-"),
        config.replace("check_interval = 3", "check_interval = 0"),
        format!("{config}  privileged = true\n"),
        config.replace("id = 42", "id = 0"),
        String::new(),
    ] {
        assert!(
            serde_json::from_value::<GitlabVmBootstrap>(json!({ "config_toml": invalid })).is_err()
        );
    }
    assert!(serde_json::from_value::<GitlabVmBootstrap>(
        json!({"config_toml": config, "token": "glrt-x"})
    )
    .is_err());
    Ok(())
}

#[test]
fn deserialization_rejects_vm_tokens_without_exclusive_forgejo_identity() -> TestResult {
    let original = serde_json::to_value(input()?)?;
    for (key, value) in [("forgejo", json!(null)), ("jit_config", json!("jit"))] {
        let mut invalid = original.clone();
        invalid[key] = value;
        assert!(serde_json::from_value::<ShaulaInputEnvelope>(invalid).is_err());
    }
    let mut invalid = original;
    invalid["generation"]["scale_set_id"] = json!(1);
    assert!(serde_json::from_value::<ShaulaInputEnvelope>(invalid).is_err());
    Ok(())
}
