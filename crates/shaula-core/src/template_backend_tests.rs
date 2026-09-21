use super::*;
use crate::fleet::FleetProviderKind::{Forgejo, Github};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn manifest(platform: &str) -> Result<ProfileManifest, Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates")
        .join(platform)
        .join("profile.yaml");
    Ok(serde_yaml::from_str(&std::fs::read_to_string(path)?)?)
}

#[test]
fn same_container_source_selects_backend_only_from_publisher_bindings() -> TestResult {
    for platform in ["docker", "kubernetes"] {
        let manifest = manifest(platform)?;
        manifest.validate_new_container_profile()?;
        let empty = Map::new();
        assert_eq!(manifest.runner_backend_for_bindings(&empty)?, "github");
        manifest.validate_bindings_for_provider(&empty, Github)?;
        assert!(manifest
            .validate_bindings_for_provider(&empty, Forgejo)
            .is_err());
        let bindings = serde_json::from_value(json!({"runner_backend":"forgejo"}))?;
        manifest.validate_bindings_for_provider(&bindings, Forgejo)?;
        assert!(manifest
            .validate_bindings_for_provider(&bindings, Github)
            .is_err());
        assert!(manifest
            .selected_runner_image(&bindings, &empty)?
            .starts_with("code.forgejo.org/forgejo/runner:"));
        assert!(manifest
            .selected_runner_image(&empty, &empty)?
            .starts_with("ghcr.io/actions/actions-runner:"));
        let parameters =
            serde_json::from_value(json!({"runner_backend":"forgejo", "runner_image":"auto"}))?;
        assert!(manifest
            .selected_runner_image(&empty, &parameters)?
            .starts_with("ghcr.io/actions/actions-runner:"));
        for value in [json!("unknown"), json!(null), json!(true)] {
            let bindings = serde_json::from_value(json!({"runner_backend":value}))?;
            assert!(manifest.runner_backend_for_bindings(&bindings).is_err());
        }
    }
    Ok(())
}

#[test]
fn explicit_image_cannot_override_the_selected_backend() -> TestResult {
    let manifest = manifest("docker")?;
    for (backend, prefix) in [("github", "ghcr.io/"), ("forgejo", "code.forgejo.org/")] {
        let bindings = serde_json::from_value(json!({"runner_backend":backend}))?;
        for image in &manifest.runner_image_digests {
            let alias = image.split('@').next().ok_or("alias missing")?;
            let parameters = serde_json::from_value(json!({"runner_image":alias}))?;
            assert_eq!(
                manifest
                    .selected_runner_image(&bindings, &parameters)
                    .is_ok(),
                image.starts_with(prefix)
            );
        }
        for alias in [json!(true), json!("custom"), json!(null)] {
            let parameters = serde_json::from_value(json!({"runner_image":alias}))?;
            assert!(manifest
                .selected_runner_image(&bindings, &parameters)
                .is_err());
        }
    }
    Ok(())
}

#[test]
fn selectable_backend_requires_both_official_images_and_container_contract() -> TestResult {
    let original = manifest("docker")?;
    for backends in [
        vec!["github"],
        vec!["forgejo", "forgejo"],
        vec!["github", "other"],
    ] {
        let mut manifest = original.clone();
        manifest.runner_backends = backends.into_iter().map(str::to_string).collect();
        assert!(manifest.validate().is_err());
    }
    let mut missing_image = original.clone();
    missing_image.runner_image_digests.pop();
    assert!(missing_image.validate().is_err());
    let mut legacy = original.clone();
    legacy.container_bootstrap_contract = None;
    assert!(legacy.validate().is_err());
    let mut vm = original;
    vm.platform = "virtual-machine".into();
    assert!(vm.validate().is_err());
    Ok(())
}

#[test]
fn image_selection_rejects_ambiguous_defaults_and_malformed_official_pins() -> TestResult {
    let mut manifest = manifest("docker")?;
    let original = manifest.runner_image_digests[0].clone();
    manifest
        .runner_image_digests
        .push(original.replace(":2.337.0@", ":2.338.0@"));
    manifest.validate()?;
    assert!(manifest
        .selected_runner_image(&Map::new(), &Map::new())
        .is_err());
    let parameters =
        serde_json::from_value(json!({"runner_image":"ghcr.io/actions/actions-runner:2.337.0"}))?;
    assert_eq!(
        manifest.selected_runner_image(&Map::new(), &parameters)?,
        original
    );
    for tag in ["", "unsafe/path", "white space"] {
        manifest.runner_image_digests[0] = original.replace(":2.337.0@", &format!(":{tag}@"));
        assert!(manifest.validate().is_err());
    }
    Ok(())
}

#[test]
fn legacy_fixed_backend_and_serialization_remain_unchanged() -> TestResult {
    let mut manifest = manifest("docker")?;
    manifest.runner_backends.clear();
    manifest
        .runner_image_digests
        .retain(|image| image.starts_with("ghcr.io/"));
    manifest.validate()?;
    let json = serde_json::to_value(&manifest)?;
    assert!(json.get("runner_backend").is_none());
    assert!(json.get("runner_backends").is_none());
    let bindings = serde_json::from_value(json!({"runner_backend":"forgejo"}))?;
    assert_eq!(manifest.runner_backend_for_bindings(&bindings)?, "github");
    assert!(manifest.validate_for_provider(Forgejo).is_err());
    Ok(())
}

#[test]
fn envelope_identity_must_match_published_selection() -> TestResult {
    use crate::template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope};
    let manifest = manifest("docker")?;
    let mut input = ShaulaInputEnvelope::new(
        GenerationIdentity {
            fleet_key: "fleet".into(),
            scale_set_id: Some(1),
            id: "gen".into(),
            runner_name: "runner".into(),
            generation_name: "generation".into(),
        },
        "jit".into(),
        BindingsDigest("digest".into()),
    );
    input.validate_for_manifest(&manifest)?;
    input
        .bindings
        .insert("runner_backend".into(), json!("forgejo"));
    assert!(input.validate_for_manifest(&manifest).is_err());
    input.forgejo = Some(crate::forgejo::ForgejoBootstrapIdentity {
        instance_url: "https://forgejo.test".into(),
        uuid: "runner-uuid".into(),
        labels: vec!["linux:host".into()],
    });
    assert!(input.validate_for_manifest(&manifest).is_err());
    input.jit_config.clear();
    input.generation.scale_set_id = None;
    input.validate_for_manifest(&manifest)?;
    input.bindings.clear();
    assert!(input.validate_for_manifest(&manifest).is_err());
    Ok(())
}
