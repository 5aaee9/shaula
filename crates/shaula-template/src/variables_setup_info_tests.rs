use super::discover_variables;
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn manifest_v2_requires_setup_info_and_v1_rejects_its_extra_field() -> TestResult {
    let directory = super::tests::fixture("", r#"{"type":"object","properties":{}}"#)?;
    let bundled = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/docker/profile.yaml");
    let v1 = std::fs::read_to_string(bundled)?;
    std::fs::write(
        directory.path().join("profile.yaml"),
        format!("{v1}\ninput_contract_version: 2\nsetup_info_contract: shaula.setup-info/v1\n"),
    )?;
    assert!(discover_variables(directory.path(), "fixture").is_err());
    let path = directory.path().join("variables.tf");
    let original = std::fs::read_to_string(&path)?;
    std::fs::write(
        &path,
        original.replace(
            "jit_config = string",
            "jit_config = string\n    setup_info = any",
        ),
    )?;
    assert!(discover_variables(directory.path(), "fixture")?.available);
    std::fs::write(directory.path().join("profile.yaml"), v1)?;
    assert!(discover_variables(directory.path(), "fixture").is_err());
    Ok(())
}
