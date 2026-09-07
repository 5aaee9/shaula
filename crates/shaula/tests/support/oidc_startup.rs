#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::process::{Child, Command};
#[path = "../../../shaula-http/tests/support/mod.rs"]
pub mod provider;

pub struct Startup {
    pub directory: tempfile::TempDir,
    pub provider: provider::Provider,
    pub port: u16,
}
impl Startup {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let provider = provider::start_provider();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = serde_json::json!({"version":1,"storage":{"data_dir":directory.path().join("data")},
            "http":{"listen":format!("127.0.0.1:{port}"),"bindings_server_key":"test-bindings-key-0123456789abcdef",
                "authorization":[{"issuer":provider.issuer,"subject":"ops","scopes":provider::SCOPES.split_whitespace().collect::<Vec<_>>()}]},
            "execution":{"engines":{"terraform":{"executable":env!("CARGO_BIN_EXE_shaula")}}}});
        std::fs::write(
            directory.path().join("bootstrap.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        std::fs::write(directory.path().join("provider.pem"), &provider.certificate).unwrap();
        Self {
            directory,
            provider,
            port,
        }
    }
    pub fn command(&self) -> Command {
        let mut command = bare_command();
        command
            .args(["serve", "--config"])
            .arg(self.directory.path().join("bootstrap.json"))
            .env("SHAULA_OIDC_PROVIDER", &self.provider.issuer)
            .env("SHAULA_OIDC_CLIENT_ID", "web")
            .env("SHAULA_OIDC_CLIENT_SECRET", "test-secret")
            .env("SHAULA_OIDC_PUBLIC_URL", "https://shaula.example")
            .env("SHAULA_OIDC_API_AUDIENCE", "api")
            .env(
                "SHAULA_OIDC_CA_CERT",
                self.directory.path().join("provider.pem"),
            );
        command
    }
    pub fn assert_failed(&self, command: &mut Command) {
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(
            !self.directory.path().join("data").exists(),
            "startup cannot create runtime state before validation"
        );
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", self.port)).is_ok(),
            "no listener after failed initialization"
        );
        for bytes in [&output.stdout, &output.stderr] {
            assert!(!String::from_utf8_lossy(bytes).contains("test-secret"));
        }
    }
}
pub fn bare_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_shaula"));
    for name in [
        "PROVIDER",
        "CLIENT_ID",
        "CLIENT_SECRET",
        "PUBLIC_URL",
        "API_AUDIENCE",
        "CA_CERT",
    ] {
        command.env_remove(format!("SHAULA_OIDC_{name}"));
    }
    command
}
pub struct Running(pub Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
