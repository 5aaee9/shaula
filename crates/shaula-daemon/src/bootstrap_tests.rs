use super::*;

/// A REAL engine file on disk: bootstrap freezes the executable to
/// an existing absolute path, so tests must point at one (R5-08).
fn engine_fixture() -> String {
    let dir = std::env::temp_dir().join("shaula-bootstrap-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("terraform-fixture.exe");
    std::fs::write(&path, b"fixture engine").unwrap();
    path.to_string_lossy().replace('\\', "/")
}

fn base_config() -> BootstrapConfig {
    serde_yaml::from_str(&format!(
        r#"
version: 1
storage:
  data_dir: /var/lib/shaula
http:
  listen: 127.0.0.1:8080
  bindings_server_key: "bootstrap-bindings-key-0123456789abcdef"
execution:
  engines:
    terraform:
      executable: "{}"
"#,
        engine_fixture()
    ))
    .unwrap()
}

#[test]
fn valid_bootstrap_freezes_absolute_engine_path() {
    let validated = ValidatedBootstrap::validate(base_config()).unwrap();
    assert_eq!(validated.listen, "127.0.0.1:8080");
    assert_eq!(
        validated.database_path,
        PathBuf::from("/var/lib/shaula/shaula.db")
    );
    assert_eq!(validated.max_active_fleets, 100);
    // The frozen authority is the ABSOLUTE fixture path (R5-08).
    assert!(validated.terraform_executable.is_absolute());
    assert!(validated.terraform_executable.is_file());
}

#[test]
fn unresolvable_engine_fails_bootstrap() {
    let mut config = base_config();
    config.execution.engines.terraform.executable = "definitely-not-on-path-xyz".to_string();
    let error = ValidatedBootstrap::validate(config).unwrap_err();
    assert!(
        error.contains("not found on PATH"),
        "a PATH-only engine that does not exist must fail bootstrap: {error}"
    );
    let mut config = base_config();
    config.execution.engines.terraform.executable = "/nonexistent/engine/terraform".to_string();
    assert!(ValidatedBootstrap::validate(config).is_err());
}

#[test]
fn template_source_import_paths_are_explicit_trusted_absolute_directories() {
    let mut config = base_config();
    assert!(ValidatedBootstrap::validate(config.clone())
        .unwrap()
        .template_source_dirs
        .is_empty());
    config.template_source_dirs = vec![PathBuf::from("relative-template-directory")];
    assert!(ValidatedBootstrap::validate(config.clone()).is_err());
    config.template_source_dirs = vec![std::env::temp_dir().join("templates")];
    assert_eq!(
        ValidatedBootstrap::validate(config.clone())
            .unwrap()
            .template_source_dirs,
        config.template_source_dirs
    );
    config.template_source_dirs = vec![std::env::temp_dir(); 17];
    assert!(ValidatedBootstrap::validate(config).is_err());
}

#[test]
fn relative_engine_path_is_frozen_absolute() {
    // R6-09: a multi-component RELATIVE path resolves against THIS
    // cwd at bootstrap; the frozen value must be its stable
    // absolute form, because the runtime later spawns from a
    // per-operation workspace where the relative spelling would no
    // longer resolve to the verified file. The fixture lives under
    // target/ so the relative spelling really does resolve from the
    // test process cwd.
    let dir = std::path::Path::new("target/test-relative-engine");
    std::fs::create_dir_all(dir).unwrap();
    let engine = dir.join("terraform.cmd");
    std::fs::write(&engine, b"@echo off\r\nexit /b 0\r\n").unwrap();
    let mut config = base_config();
    config.execution.engines.terraform.executable =
        "target/test-relative-engine/terraform.cmd".to_string();
    let validated = ValidatedBootstrap::validate(config).unwrap();
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(
        validated.terraform_executable,
        cwd.join(&engine),
        "a relative engine path must be frozen to its stable absolute form"
    );
    std::fs::remove_file(&engine).ok();
    std::fs::remove_dir(dir).ok();
}

#[test]
fn non_loopback_listen_fails_closed() {
    let mut config = base_config();
    config.http.listen = "0.0.0.0:8080".to_string();
    assert!(ValidatedBootstrap::validate(config).is_err());
}

#[test]
fn missing_observability_section_defaults_service_name() {
    let config = base_config();
    assert_eq!(config.observability.service_name, "shaula");
}

#[test]
fn unknown_fields_rejected() {
    let raw = r#"
version: 1
storage:
  data_dir: /tmp
http:
  listen: 127.0.0.1:8080
  bindings_server_key: "bootstrap-bindings-key-0123456789abcdef"
template_profiles:
  - key: kubernetes
"#;
    let parsed: Result<BootstrapConfig, _> = serde_yaml::from_str(raw);
    assert!(
        parsed.is_err(),
        "bootstrap must not carry resource catalogs"
    );
}

#[test]
fn traversal_paths_rejected() {
    let mut config = base_config();
    config.storage.work_root = "../escape".to_string();
    assert!(ValidatedBootstrap::validate(config).is_err());
}

#[test]
fn legacy_backend_token_rejected() {
    let parsed = serde_yaml::from_str::<crate::config::HttpConfigDto>(
        "listen: 127.0.0.1:8080\nbackend_token: obsolete\nbindings_server_key: key\n",
    );
    assert!(parsed.is_err());
}

#[test]
fn size_parsing() {
    assert_eq!(parse_size("1MiB"), Ok(1024 * 1024));
    assert_eq!(parse_size("64MiB"), Ok(64 * 1024 * 1024));
    assert_eq!(parse_size("512KiB"), Ok(512 * 1024));
    assert_eq!(parse_size("1024B"), Ok(1024));
    assert!(parse_size("12GB").is_err());
}

#[test]
fn otlp_endpoint_is_optional_and_passes_through_validation() {
    let validated = ValidatedBootstrap::validate(base_config()).unwrap();
    assert_eq!(
        validated.otlp_endpoint, None,
        "a missing otlp section keeps local-only telemetry"
    );
    let mut config = base_config();
    config.observability.otlp.endpoint = Some("http://127.0.0.1:4318".to_string());
    let validated = ValidatedBootstrap::validate(config).unwrap();
    assert_eq!(
        validated.otlp_endpoint.as_deref(),
        Some("http://127.0.0.1:4318")
    );
}

#[test]
fn otlp_protocol_other_than_http_json_fails_bootstrap() {
    let mut config = base_config();
    config.observability.otlp.protocol = "grpc".to_string();
    let error = ValidatedBootstrap::validate(config).unwrap_err();
    assert!(
        error.contains("http/json"),
        "the exporter speaks OTLP/HTTP http/json only: {error}"
    );
}

#[test]
fn otlp_endpoint_must_be_plain_http() {
    let mut config = base_config();
    config.observability.otlp.endpoint = Some("https://collector:4318".to_string());
    let error = ValidatedBootstrap::validate(config).unwrap_err();
    assert!(
        error.contains("http://"),
        "TLS terminates at the collector boundary, not in the exporter: {error}"
    );
}

#[test]
fn observability_dead_metrics_and_trace_config_rejected() {
    // The traces/metrics knobs died with the unconsumed registry: unknown
    // observability keys must fail closed, not be silently ignored.
    let parsed: Result<BootstrapConfig, _> = serde_yaml::from_str(&format!(
        r#"
version: 1
storage:
  data_dir: /var/lib/shaula
http:
  listen: 127.0.0.1:8080
  bindings_server_key: "bootstrap-bindings-key-0123456789abcdef"
execution:
  engines:
    terraform:
      executable: "{}"
observability:
  metrics:
    interval_secs: 15
"#,
        engine_fixture()
    ));
    assert!(
        parsed.is_err(),
        "observability.metrics is no longer a config field"
    );
}
