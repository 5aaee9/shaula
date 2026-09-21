use super::discover_variables;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn vm_contract_alone_admits_optional_protected_bootstrap_member() -> TestResult {
    let directory = super::tests::fixture("", r#"{"type":"object","properties":{}}"#)?;
    let sources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
    let path = directory.path().join("variables.tf");
    let original = std::fs::read_to_string(&path)?;
    for platform in ["proxmox", "aws", "tencentcloud", "alicloud"] {
        std::fs::copy(
            sources.join(platform).join("profile.yaml"),
            directory.path().join("profile.yaml"),
        )?;
        for member in [
            "",
            "forgejo_vm = any",
            "forgejo_vm = optional(any, null)",
            "forgejo_vm = optional(string)",
        ] {
            std::fs::write(
                &path,
                original.replace(
                    "jit_config = string",
                    &format!("jit_config = string\n    forgejo = optional(any)\n    {member}"),
                ),
            )?;
            assert!(
                discover_variables(directory.path(), "fixture").is_err(),
                "{platform}: {member}"
            );
        }
        std::fs::write(
            &path,
            original.replace(
                "jit_config = string",
                "jit_config = string\n    forgejo = optional(any)\n    forgejo_vm = optional(any)",
            ),
        )?;
        assert!(discover_variables(directory.path(), "fixture")?.available);
        let variables = discover_variables(&sources.join(platform), "fixture")?;
        let backend = variables
            .bindings
            .iter()
            .find(|field| field.key == "runner_backend")
            .ok_or("backend missing")?;
        assert_eq!(backend.default_value_json.as_deref(), Some("\"github\""));
        assert_eq!(
            backend
                .options
                .iter()
                .map(|option| option.value_json.as_str())
                .collect::<Vec<_>>(),
            ["\"github\"", "\"forgejo\""]
        );
        assert!(!backend.required && !backend.sensitive);
        assert!(!variables
            .parameters
            .iter()
            .any(|field| field.key == "runner_backend"));
    }
    for platform in ["docker", "kubernetes"] {
        std::fs::copy(
            sources.join(platform).join("profile.yaml"),
            directory.path().join("profile.yaml"),
        )?;
        assert!(discover_variables(directory.path(), "fixture").is_err());
    }
    Ok(())
}

#[test]
fn selectable_backend_requires_optional_nonsecret_identity_without_a_default() -> TestResult {
    let directory = super::tests::fixture("", r#"{"type":"object","properties":{}}"#)?;
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/docker/profile.yaml");
    std::fs::copy(source, directory.path().join("profile.yaml"))?;
    let path = directory.path().join("variables.tf");
    let original = std::fs::read_to_string(&path)?;
    assert!(discover_variables(directory.path(), "fixture").is_err());
    for declaration in ["any", "optional(string)", "optional(any, null)"] {
        std::fs::write(
            &path,
            original.replace(
                "jit_config = string",
                &format!("jit_config = string\n    forgejo = {declaration}"),
            ),
        )?;
        assert!(discover_variables(directory.path(), "fixture").is_err());
    }
    std::fs::write(
        &path,
        original.replace(
            "jit_config = string",
            "jit_config = string\n    forgejo = optional(any)",
        ),
    )?;
    assert!(discover_variables(directory.path(), "fixture")?.available);
    Ok(())
}

#[test]
fn provider_manifest_controls_the_nonsecret_forgejo_envelope_member() -> TestResult {
    let directory = super::tests::fixture("", r#"{"type":"object","properties":{}}"#)?;
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/docker/profile.yaml");
    let mut manifest: shaula_core::template::ProfileManifest =
        serde_yaml::from_str(&std::fs::read_to_string(source)?)?;
    manifest.runner_backends.clear();
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
