use super::*;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn command(config: &Path, origin: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_shaula"));
    command.args([
        "--client-config",
        &config.to_string_lossy(),
        "--server",
        origin,
        "--allow-loopback-http",
        "--output",
        "json",
    ]);
    command
        .env_remove("SHAULA_ACCESS_TOKEN")
        .env_remove("SHAULA_CONTEXT");
    command
}

#[test]
fn invalid_arguments_have_a_stable_envelope_without_echoing_secret_argv() -> TestResult {
    let output = Command::new(env!("CARGO_BIN_EXE_shaula"))
        .args(["auth", "whoami", "--token", "do-not-echo-this-secret"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let body: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(body.as_object().ok_or("object")?.len(), 6);
    assert_eq!(body["error"]["code"], 2);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("do-not-echo"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("do-not-echo"));
    Ok(())
}
#[tokio::test]
async fn cli_login_context_whoami_and_explicit_remote_logout() -> TestResult {
    let fixture = Fixture::new().await?;
    let TokenIssue::Issued { secret, .. } = fixture
        .primary("ops")?
        .access_tokens()
        .issue(&request(vec!["fleet.read".into()]), "login-cli".into())
        .await?
    else {
        return Err("secret".into());
    };
    let directory = tempfile::tempdir()?;
    let config = directory.path().join("private/client.toml");
    let origin = fixture.origin.clone();
    let (output, whoami, logout) = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
        let mut child = command(&config, &origin)
            .args(["--context", "test", "auth", "login", "--token-stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("stdin"))?
            .write_all(secret.expose().as_bytes())?;
        let output = child.wait_with_output()?;
        let whoami = command(&config, &origin)
            .args(["auth", "whoami"])
            .output()?;
        let logout = command(&config, &origin)
            .args(["auth", "logout", "--revoke"])
            .output()?;
        Ok((output, whoami, logout))
    })
    .await??;
    for (output, name) in [
        (output, "auth.login"),
        (whoami, "auth.whoami"),
        (logout, "auth.logout"),
    ] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let body: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(body["command"], name);
        assert_eq!(body.as_object().ok_or("object")?.len(), 6);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("shaula_pat_v1_"));
    }
    Ok(())
}
