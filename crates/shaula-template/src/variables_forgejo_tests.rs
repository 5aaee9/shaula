use super::discover_variables;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn provider_manifest_controls_the_nonsecret_forgejo_envelope_member() -> TestResult {
    let directory = super::tests::fixture("", r#"{"type":"object","properties":{}}"#)?;
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/docker/profile.yaml");
    let mut manifest: shaula_core::template::ProfileManifest =
        serde_yaml::from_str(&std::fs::read_to_string(source)?)?;
    manifest.runner_backend = "forgejo".into();
    manifest.runner_image_digests = vec![format!(
        "code.forgejo.org/forgejo/runner:13.1.0@sha256:{}",
        "a".repeat(64)
    )];
    let manifest_path = directory.path().join("profile.yaml");
    std::fs::write(&manifest_path, serde_yaml::to_string(&manifest)?)?;
    assert!(discover_variables(directory.path(), "fixture").is_err());
    let variables = directory.path().join("variables.tf");
    let original = std::fs::read_to_string(&variables)?;
    std::fs::write(
        &variables,
        original.replace(
            "jit_config = string",
            "jit_config = string\n    forgejo = any",
        ),
    )?;
    assert!(discover_variables(directory.path(), "fixture")?.available);
    // A legacy/GitHub manifest must still refuse the additional system member.
    std::fs::remove_file(&manifest_path)?;
    assert!(discover_variables(directory.path(), "fixture").is_err());
    Ok(())
}
