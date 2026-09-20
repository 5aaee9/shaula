use super::*;
use serde_json::json;
use shaula_core::registry::BindingsSchema;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn template() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/docker")
}

fn manifest() -> Result<ProfileManifest, Box<dyn std::error::Error>> {
    Ok(crate::manifest::parse_manifest(&std::fs::read_to_string(
        template().join("profile.yaml"),
    )?)?)
}

fn bindings(value: Value) -> Result<Map<String, Value>, Box<dyn std::error::Error>> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "not an object".into())
}

fn schema() -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&std::fs::read(
        template().join("schemas/bindings.schema.json"),
    )?)?)
}

#[test]
fn docker_ssh_endpoints_are_strict_and_support_custom_ports_and_ipv6() -> TestResult {
    assert_eq!(
        schema()?["properties"]["docker_host"]["anyOf"][2]["pattern"],
        SSH_HOST_PATTERN
    );
    for host in [
        "ssh://runner@host",
        "ssh://runner@host:2222",
        "ssh://runner@[2001:db8::1]:2222",
        "ssh://user.name@192.0.2.1:22",
    ] {
        assert!(ssh_host(host), "{host}");
    }
    for host in [
        "ssh://host",
        "ssh://user:password@host",
        "ssh://user@host/path",
        "ssh://user@host?command=foo",
        "ssh://user@host#fragment",
        "ssh://user@host:0",
        "ssh://user@host:65536",
        "ssh://-oProxyCommand=evil@host",
        "ssh://user@-bad",
        "ssh://user@host\n",
        "ssh://user%20name@host",
        "tcp://host:2375",
    ] {
        assert!(!ssh_host(host), "{host}");
    }
    Ok(())
}

#[test]
fn docker_bindings_accept_each_auth_method_and_preserve_local_defaults() -> TestResult {
    let schema = BindingsSchema::parse(&schema()?)?;
    for value in [
        json!({"docker_host":"unix:///var/run/docker.sock"}),
        json!({"docker_host":"npipe:////./pipe/docker_engine"}),
        json!({"docker_host":"ssh://runner@host:2222", "ssh_password":"secret"}),
        json!({"docker_host":"ssh://runner@[::1]:2222", "ssh_private_key":"private-key"}),
        json!({"docker_host":"ssh://runner@host", "ssh_private_key":"private-key",
            "ssh_private_key_passphrase":"passphrase", "ssh_known_hosts":"host ssh-ed25519 key\n"}),
    ] {
        schema.validate(&bindings(value)?)?;
    }
    assert!(DockerSsh::prepare(&manifest()?, &Map::new())?.is_none());
    Ok(())
}

#[test]
fn docker_bindings_reject_ambiguous_or_misplaced_secrets() -> TestResult {
    let schema = BindingsSchema::parse(&schema()?)?;
    let manifest = manifest()?;
    for value in [
        json!({"docker_host":"tcp://host:2375"}),
        json!({"docker_host":"ssh://runner@host"}),
        json!({"docker_host":"ssh://runner@host", "ssh_password":"password", "ssh_private_key":"key"}),
        json!({"docker_host":"unix:///socket", "ssh_password":"password"}),
        json!({"docker_host":"unix:///socket", "ssh_known_hosts":"hosts"}),
        json!({"docker_host":"ssh://runner@host", "ssh_password":"password", "ssh_private_key_passphrase":"phrase"}),
        json!({"docker_host":"ssh://runner@host", "ssh_password":""}),
        json!({"docker_host":"ssh://runner@host", "ssh_password":"secret\nsecond-line"}),
        json!({"docker_host":"ssh://runner@host", "ssh_password":null}),
    ] {
        let bindings = bindings(value)?;
        assert!(schema.validate(&bindings).is_err());
        assert!(DockerSsh::prepare(&manifest, &bindings).is_err());
    }
    Ok(())
}

#[test]
fn ssh_credentials_are_discoverable_but_never_readable() -> TestResult {
    let variables = crate::variables::discover_variables(&template(), "digest")?;
    let schema = BindingsSchema::parse(&schema()?)?;
    let values = bindings(
        json!({"ssh_password":"password-secret", "ssh_private_key":"key-secret",
        "ssh_private_key_passphrase":"phrase-secret"}),
    )?;
    for name in [
        "ssh_password",
        "ssh_private_key",
        "ssh_private_key_passphrase",
    ] {
        let field = variables
            .bindings
            .iter()
            .find(|field| field.key == name)
            .ok_or("missing field")?;
        assert!(field.sensitive && !field.required);
        assert!(field.default_value_json.is_none() && field.options.is_empty());
        assert!(schema.sensitive(name));
    }
    let projected = serde_json::to_string(&schema.project(&values))?;
    assert!(!projected.contains("-secret"));
    assert!(!schema.sensitive("docker_host"));
    assert!(!schema.sensitive("ssh_known_hosts"));
    Ok(())
}

fn connection(key: Option<&str>, secret: Option<&str>) -> std::io::Result<DockerSsh> {
    DockerSsh::materialize(
        &serde_json::from_value(json!({"ssh_known_hosts":"host ssh-ed25519 public-key\n"}))?,
        key,
        secret,
        &std::env::current_exe()?,
        &std::env::current_exe()?,
    )
}

#[test]
fn ssh_files_are_private_transient_and_only_paths_reach_child_environment() -> TestResult {
    let connection = connection(Some("private-key\r\nbody"), Some("private-passphrase"))?;
    let root = connection._directory.path().to_path_buf();
    let config: Config = serde_json::from_slice(&std::fs::read(root.join("connection.json"))?)?;
    assert_eq!(
        std::fs::read_to_string(root.join("identity"))?,
        "private-key\nbody\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("credential"))?,
        "private-passphrase"
    );
    let config_json = serde_json::to_string(&config)?;
    assert!(!config_json.contains("private-passphrase") && !config_json.contains("private-key"));
    assert!(config
        .arguments
        .iter()
        .any(|arg| arg == "StrictHostKeyChecking=yes"));
    assert!(config
        .arguments
        .iter()
        .any(|arg| arg == "PreferredAuthentications=publickey"));
    assert!(config
        .arguments
        .iter()
        .any(|arg| arg == "IdentityAgent=none"));
    let mut environment = vec![
        ("TF_HTTP_PASSWORD".into(), "backend-secret".into()),
        ("PATH".into(), "untrusted-path".into()),
        ("SSH_AUTH_SOCK".into(), "ambient-agent".into()),
    ];
    connection.configure(&mut environment);
    let bootstrap = bootstrap_environment(&environment)?;
    assert_eq!(bootstrap.len(), ENVIRONMENT_KEYS.len());
    assert!(!bootstrap.iter().any(|(_, value)| value.contains("secret")
        || value == "untrusted-path"
        || value == "ambient-agent"));
    assert!(environment.iter().any(|(key, _)| key == "TF_HTTP_PASSWORD"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&root)?.permissions().mode() & 0o777,
            0o700
        );
        for name in [
            "config",
            "identity",
            "credential",
            "connection.json",
            "known_hosts",
        ] {
            assert_eq!(
                std::fs::metadata(root.join(name))?.permissions().mode() & 0o777,
                0o600
            );
        }
    }
    drop(connection);
    assert!(!root.exists());
    assert!(bootstrap_environment(&[]).is_err());
    Ok(())
}

#[test]
fn password_and_unencrypted_key_modes_do_not_fall_back_to_other_identities() -> TestResult {
    for (key, secret, method, batch) in [
        (None, Some("password"), "password", "no"),
        (Some("key"), None, "publickey", "yes"),
    ] {
        let connection = connection(key, secret)?;
        let config: Config = serde_json::from_slice(&std::fs::read(
            connection._directory.path().join("connection.json"),
        )?)?;
        assert_eq!(config.secret_file.is_some(), secret.is_some());
        assert!(config
            .arguments
            .contains(&format!("PreferredAuthentications={method}")));
        assert!(config.arguments.contains(&format!("BatchMode={batch}")));
    }
    Ok(())
}

#[test]
fn each_invocation_rebuilds_disposable_paths_from_the_same_frozen_bindings() -> TestResult {
    let first = connection(Some("key"), Some("phrase"))?;
    let second = connection(Some("key"), Some("phrase"))?;
    assert_ne!(first._directory.path(), second._directory.path());
    drop(first);
    assert!(second._directory.path().join("identity").exists());
    Ok(())
}
