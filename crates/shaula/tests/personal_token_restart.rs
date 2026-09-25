//! Production serve restart and OIDC outage boundaries for local PATs.
#[path = "support/oidc_startup.rs"]
mod support;
use shaula_client::{
    types::{IssueToken, TokenIssue},
    Client, Secret,
};
use shaula_core::registry::Scope;
use std::process::Stdio;
use support::{provider, Running, Startup};
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn configure(fixture: &Startup, enabled: bool, grant: bool) -> TestResult {
    let path = fixture.directory.path().join("bootstrap.json");
    let mut config: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    config["http"]["access_tokens"] = serde_json::json!({"enabled":enabled});
    config["http"]["authorization"] = if grant {
        serde_json::json!([{"issuer":fixture.provider.issuer,"subject":"ops","scopes":Scope::ALL.iter().map(|s|s.as_str()).collect::<Vec<_>>()}])
    } else {
        serde_json::json!([])
    };
    std::fs::write(path, serde_json::to_vec(&config)?)?;
    Ok(())
}
async fn start(fixture: &Startup) -> Result<Running, Box<dyn std::error::Error>> {
    let mut child = Running(
        fixture
            .command()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    for _ in 0..100 {
        if child.0.try_wait()?.is_some() {
            return Err("serve exited during startup".into());
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", fixture.port))
            .await
            .is_ok()
        {
            return Ok(child);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err("serve startup timeout".into())
}
fn stop(mut child: Running) -> TestResult {
    child.0.kill()?;
    child.0.wait()?;
    Ok(())
}

#[tokio::test]
async fn pat_survives_runtime_provider_outage_but_not_disabled_or_removed_grant_restart(
) -> TestResult {
    let fixture = Startup::new();
    configure(&fixture, true, true)?;
    let child = start(&fixture).await?;
    let origin = format!("http://127.0.0.1:{}", fixture.port);
    let scopes = Scope::ALL
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let jwt = provider::sign(
        provider::claims(&fixture.provider.issuer, "api", "ops", &scopes),
        "at+jwt",
        "test-key",
    );
    let primary = Client::loopback(&origin, Secret::new(jwt))?;
    let TokenIssue::Issued { secret, metadata } = primary
        .access_tokens()
        .issue(
            &IssueToken {
                name: "restart".into(),
                scopes: vec!["fleet.read".into()],
                expires_in_seconds: Some(3600),
            },
            "restart-key".into(),
        )
        .await?
    else {
        return Err("issued".into());
    };
    let pat = Client::loopback(&origin, secret)?;
    fixture
        .provider
        .control
        .lock()
        .map_err(|_| "provider lock")?
        .offline = true;
    assert!(pat.fleets().list_typed().await?.data.fleets.is_empty());
    stop(child)?;
    // A persisted PAT must never bypass Discovery at serve startup.
    let mut command = fixture.command();
    let failed = tokio::task::spawn_blocking(move || command.output()).await??;
    assert!(!failed.status.success());
    fixture
        .provider
        .control
        .lock()
        .map_err(|_| "provider lock")?
        .offline = false;
    let child = start(&fixture).await?;
    assert!(pat.fleets().list_typed().await?.data.fleets.is_empty());
    assert_eq!(
        primary.access_tokens().get(&metadata.id).await?.data.state,
        "active"
    );
    stop(child)?;
    configure(&fixture, false, true)?;
    let child = start(&fixture).await?;
    assert!(matches!(
        pat.session().await,
        Err(shaula_client::Error::Http { status: 401, .. })
    ));
    assert_eq!(
        primary.access_tokens().get(&metadata.id).await?.data.state,
        "disabled"
    );
    stop(child)?;
    configure(&fixture, true, false)?;
    let child = start(&fixture).await?;
    assert!(matches!(
        pat.session().await,
        Err(shaula_client::Error::Http { status: 401, .. })
    ));
    stop(child)?;
    Ok(())
}
