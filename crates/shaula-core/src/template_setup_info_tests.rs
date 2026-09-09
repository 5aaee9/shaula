use super::*;
use crate::template::{BindingsDigest, GenerationIdentity, ProfileManifest, ShaulaInputEnvelope};
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn input() -> ShaulaInputEnvelope {
    ShaulaInputEnvelope::new(
        GenerationIdentity {
            fleet_key: "fleet".into(),
            scale_set_id: 1,
            id: "generation".into(),
            runner_name: "runner".into(),
            generation_name: "resource".into(),
        },
        "jit-secret".into(),
        BindingsDigest("binding".into()),
    )
}

fn descriptor() -> CoreResult<SetupInfoDescriptor> {
    SetupInfoDescriptor::enabled(
        "https://logs.test/runner/v1/generations/generation/setup-info".into(),
        "secret_capability_marker_aaaaaaaaaaaaaaaa".into(),
        2_000_000_000,
        60,
    )
}

#[test]
fn v1_round_trip_omits_setup_info_and_v2_requires_explicit_descriptor() -> TestResult {
    let v1 = serde_json::to_value(input())?;
    assert!(v1.get("setup_info").is_none());
    assert_eq!(
        serde_json::from_value::<ShaulaInputEnvelope>(v1.clone())?,
        input()
    );
    let v2 = input().with_setup_info(SetupInfoDescriptor::Disabled)?;
    assert_eq!(v2.contract_version, 2);
    assert_eq!(
        serde_json::from_value::<ShaulaInputEnvelope>(serde_json::to_value(&v2)?)?,
        v2
    );
    for (version, setup) in [
        (1, Some(serde_json::json!({"status":"disabled"}))),
        (1, Some(serde_json::Value::Null)),
        (2, None),
        (3, None),
    ] {
        let mut invalid = v1.clone();
        invalid["contract_version"] = version.into();
        if let Some(value) = setup {
            invalid["setup_info"] = value;
        }
        assert!(serde_json::from_value::<ShaulaInputEnvelope>(invalid).is_err());
    }
    Ok(())
}

#[test]
fn capability_debug_is_redacted_and_generation_cannot_be_substituted() -> TestResult {
    let mut value = input().with_setup_info(descriptor()?)?;
    value
        .parameters
        .insert("secret".into(), "parameter-secret-marker".into());
    let debug = format!("{value:?}");
    assert!(!debug.contains("secret_capability_marker"));
    assert!(!debug.contains("https://logs.test"));
    assert!(!debug.contains("jit-secret"));
    assert!(!debug.contains("parameter-secret-marker"));
    let mut wrong = input();
    wrong.generation.id = "other".into();
    assert!(wrong.with_setup_info(descriptor()?).is_err());
    Ok(())
}

#[test]
fn manifest_requires_exact_input_and_setup_contract_pair() -> TestResult {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/docker/profile.yaml");
    let mut manifest: ProfileManifest = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
    let original = serde_json::to_value(&manifest)?;
    assert!(original.get("input_contract_version").is_none());
    input().validate_for_manifest(&manifest)?;
    manifest.input_contract_version = 2;
    assert!(manifest.validate().is_err());
    manifest.setup_info_contract = Some(SETUP_INFO_CONTRACT.into());
    manifest.validate()?;
    assert!(input().validate_for_manifest(&manifest).is_err());
    input()
        .with_setup_info(SetupInfoDescriptor::Disabled)?
        .validate_for_manifest(&manifest)?;
    manifest.input_contract_version = 3;
    assert!(manifest.validate().is_err());
    Ok(())
}
