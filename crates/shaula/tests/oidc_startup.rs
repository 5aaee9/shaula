#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "support/oidc_startup.rs"]
mod support;
use support::*;

#[test]
fn oidc_tests_help_version_and_missing_configuration() {
    for args in [vec!["--help"], vec!["version"], vec!["serve", "--help"]] {
        let output = bare_command().args(args).output().unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("--oidc-client-secret"));
    }
    let fixture = Startup::new();
    for name in [
        "PROVIDER",
        "CLIENT_ID",
        "CLIENT_SECRET",
        "PUBLIC_URL",
        "API_AUDIENCE",
    ] {
        fixture.assert_failed(fixture.command().env_remove(format!("SHAULA_OIDC_{name}")));
    }
}

#[test]
fn oidc_tests_invalid_and_explicit_empty_values_never_fall_back() {
    let fixture = Startup::new();
    for provider in [
        "",
        "http://localhost",
        "https://user:password@example.com",
        "https://issuer.example?query",
        "https://issuer.example#fragment",
    ] {
        fixture.assert_failed(fixture.command().args(["--oidc-provider", provider]));
    }
    for (key, value) in [
        ("CLIENT_ID", ""),
        ("CLIENT_SECRET", ""),
        ("PUBLIC_URL", "https://shaula.example/prefix"),
        ("API_AUDIENCE", "web"),
    ] {
        fixture.assert_failed(fixture.command().env(format!("SHAULA_OIDC_{key}"), value));
    }
    assert_eq!(fixture.provider.control.lock().unwrap().discovery_reads, 0);
}

#[test]
fn oidc_tests_discovery_failure_prevents_listener_and_workers() {
    let fixture = Startup::new();
    fixture
        .provider
        .control
        .lock()
        .unwrap()
        .metadata
        .insert("issuer".into(), serde_json::json!("https://wrong.example"));
    fixture.assert_failed(&mut fixture.command());
    fixture.provider.control.lock().unwrap().metadata.clear();
    fixture.provider.control.lock().unwrap().keys = Some(serde_json::json!({"keys":[]}));
    fixture.assert_failed(&mut fixture.command());
    fixture.provider.control.lock().unwrap().offline = true;
    fixture.assert_failed(
        fixture
            .command()
            .env("SHAULA_OIDC_PROVIDER", "invalid-env")
            .args(["--oidc-provider", &fixture.provider.issuer]),
    );
    assert!(
        fixture.provider.control.lock().unwrap().discovery_reads >= 3,
        "CLI overrides env and reaches the intended provider"
    );
}
