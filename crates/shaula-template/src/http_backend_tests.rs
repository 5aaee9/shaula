use super::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn config() -> TestResult<HttpBackendConfig> {
    Ok(HttpBackendConfig::new(
        "127.0.0.1:8081".parse()?,
        Uuid::new_v4(),
        StateCapability::issue(),
    )?)
}

fn materialize(dir: &Path) -> TestResult {
    std::fs::write(
        dir.join("main.tf"),
        "resource \"terraform_data\" \"runner\" {}\n",
    )?;
    std::fs::write(dir.join("profile.yaml"), "contract_version: 1\n")?;
    std::fs::write(dir.join(".terraform.lock.hcl"), "")?;
    Ok(())
}

#[test]
fn only_fixed_system_backend_file_is_outside_the_artifact_commitment() -> TestResult {
    let tmp = tempfile::tempdir()?;
    materialize(tmp.path())?;
    let before = crate::workspace::template_files_digest(tmp.path())?;
    let config = config()?;
    config.install(tmp.path())?;
    config.verify(tmp.path(), false)?;
    assert_eq!(
        before,
        crate::workspace::http_template_files_digest(tmp.path())?
    );
    assert_ne!(before, crate::workspace::template_files_digest(tmp.path())?);
    assert!(!std::fs::read_to_string(tmp.path().join(BACKEND_FILE))?
        .contains(config.capability.expose()));
    assert!(
        config.install(tmp.path()).is_err(),
        "reserved file must not be overwritten"
    );
    std::fs::write(
        tmp.path().join("shaula.backend_extra.tf"),
        "resource \"x\" \"extra\" {}",
    )?;
    assert_ne!(
        before,
        crate::workspace::http_template_files_digest(tmp.path())?
    );
    std::fs::write(
        tmp.path().join(BACKEND_FILE),
        "terraform { backend \"http\" { address = \"http://evil\" } }",
    )?;
    assert!(crate::workspace::http_template_files_digest(tmp.path()).is_err());
    Ok(())
}

#[test]
fn backend_and_control_credentials_cannot_be_overridden_by_provider_environment() -> TestResult {
    let config = config()?;
    let env = config.environment(&[("DOCKER_HOST".into(), "unix:///socket".into())])?;
    let get = |name: &str| {
        env.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(get("TF_HTTP_LOCK_METHOD"), Some("LOCK"));
    assert_eq!(get("TF_HTTP_UNLOCK_METHOD"), Some("UNLOCK"));
    assert_eq!(get("TF_HTTP_UPDATE_METHOD"), Some("POST"));
    assert_eq!(get("TF_HTTP_PASSWORD"), Some(config.capability.expose()));
    assert_eq!(get("TF_HTTP_ADDRESS"), get("TF_HTTP_LOCK_ADDRESS"));
    assert_eq!(get("TF_HTTP_ADDRESS"), get("TF_HTTP_UNLOCK_ADDRESS"));
    assert!(!format!("{config:?}").contains(config.capability.expose()));
    for name in [
        "TF_HTTP_ADDRESS",
        "tf_http_password",
        "TF_CLI_ARGS",
        "TF_CLI_ARGS_plan",
        "TF_WORKSPACE",
        "TF_DATA_DIR",
        "SHAULA_CONTROL_TOKEN",
        "GITHUB_TOKEN",
        "OIDC_SECRET",
        "https_proxy",
    ] {
        assert!(config
            .environment(&[(name.into(), "untrusted".into())])
            .is_err());
    }
    assert!(config
        .environment(&[
            ("DOCKER_HOST".into(), "a".into()),
            ("docker_host".into(), "b".into())
        ])
        .is_err());
    assert!(HttpBackendConfig::new(
        "0.0.0.0:8081".parse()?,
        Uuid::new_v4(),
        StateCapability::issue()
    )
    .is_err());
    assert!(HttpBackendConfig::new(
        "127.0.0.1:0".parse()?,
        Uuid::new_v4(),
        StateCapability::issue()
    )
    .is_err());
    assert!(config
        .check_generation(&Uuid::new_v4().to_string())
        .is_err());
    Ok(())
}

#[test]
fn template_backend_and_cloud_overrides_are_rejected_in_hcl_and_json() -> TestResult {
    for (filename, content) in [
        ("extra.tf", "terraform { backend \"s3\" {} }"),
        (
            "override.tf",
            "terraform { backend \"http\" { address = \"http://evil\" } }",
        ),
        ("cloud.tf", "terraform { cloud {} }"),
        (
            "override.tf.json",
            r#"{"terraform":{"back\u0065nd":{"http":{"address":"http://evil"}}}}"#,
        ),
        (BACKEND_FILE, std::str::from_utf8(BACKEND_HCL)?),
    ] {
        let tmp = tempfile::tempdir()?;
        materialize(tmp.path())?;
        std::fs::write(tmp.path().join(filename), content)?;
        assert!(config()?.install(tmp.path()).is_err());
    }
    Ok(())
}

#[test]
fn existing_and_emergency_state_are_preserved_and_never_migrated_implicitly() -> TestResult {
    for name in [
        "terraform.tfstate",
        "terraform.tfstate.backup",
        "errored.tfstate",
    ] {
        let tmp = tempfile::tempdir()?;
        materialize(tmp.path())?;
        std::fs::write(tmp.path().join(name), "unique-state-evidence")?;
        assert!(config()?.install(tmp.path()).is_err());
        assert!(fresh_workspace(tmp.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(name))?,
            "unique-state-evidence"
        );
    }
    let tmp = tempfile::tempdir()?;
    materialize(tmp.path())?;
    let config = config()?;
    config.install(tmp.path())?;
    std::fs::create_dir(tmp.path().join(".terraform"))?;
    let metadata = tmp.path().join(".terraform/terraform.tfstate");
    std::fs::write(&metadata, r#"{"backend":{"type":"local"}}"#)?;
    assert!(config.verify(tmp.path(), true).is_err());
    std::fs::write(
        &metadata,
        r#"{"backend":{"type":"http","config":{"address":"http://evil"}}}"#,
    )?;
    assert!(config.verify(tmp.path(), true).is_err());
    std::fs::write(&metadata, r#"{"backend":{"type":"http","config":{}}}"#)?;
    config.verify(tmp.path(), true)?;
    std::fs::write(tmp.path().join("errored.tfstate"), "new-emergency-state")?;
    assert!(config.verify(tmp.path(), true).is_err());
    Ok(())
}

#[test]
#[cfg(unix)]
fn system_file_cannot_be_a_symlink_or_writable_hardlink() -> TestResult {
    let root = tempfile::tempdir()?;
    materialize(root.path())?;
    let other = tempfile::NamedTempFile::new()?;
    std::fs::write(other.path(), BACKEND_HCL)?;
    let target = root.path().join(BACKEND_FILE);
    std::os::unix::fs::symlink(other.path(), &target)?;
    assert!(verify_system_file(root.path()).is_err());
    std::fs::remove_file(&target)?;
    std::fs::hard_link(other.path(), &target)?;
    assert!(verify_system_file(root.path()).is_err());
    Ok(())
}
