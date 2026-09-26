//! Exercises the actual serve composition, including mandatory OIDC, listeners,
//! migration gating and bounded shutdown. Linux cgroup delegation is required.
#![cfg(target_os = "linux")]
#[path = "support/oidc_startup.rs"]
mod support;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use std::{process::Stdio, time::Duration};
use support::{provider, Running, Startup};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn token(fixture: &Startup) -> String {
    provider::sign(
        provider::claims(&fixture.provider.issuer, "api", "ops", provider::SCOPES),
        "at+jwt",
        "test-key",
    )
}

fn configure(fixture: &Startup) -> TestResult {
    let path = fixture.directory.path().join("bootstrap.json");
    let mut config: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    config["lifecycle"]["cgroup_root"] = std::env::var("SHAULA_TEST_CGROUP")?.into();
    std::fs::write(path, serde_json::to_vec(&config)?)?;
    Ok(())
}

async fn launch(fixture: &Startup) -> TestResult<Running> {
    let mut child = Running(fixture.command().stdout(Stdio::null()).spawn()?);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()?;
    for _ in 0..150 {
        if client
            .get(format!("http://127.0.0.1:{}/livez", fixture.port))
            .bearer_auth(token(fixture))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return Ok(child);
        }
        if let Some(status) = child.0.try_wait()? {
            return Err(format!("serve exited: {status}").into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("serve did not become live".into())
}

async fn stop(mut child: Running) -> TestResult {
    assert!(std::process::Command::new("kill")
        .args(["-INT", &child.0.id().to_string()])
        .status()?
        .success());
    for _ in 0..150 {
        if let Some(status) = child.0.try_wait()? {
            assert!(status.success(), "graceful shutdown failed: {status}");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("serve failed to stop within 15 seconds".into())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn serve_binds_private_and_management_then_stops_and_restarts() -> TestResult {
    let fixture = Startup::new();
    configure(&fixture)?;
    for _ in 0..2 {
        let child = launch(&fixture).await?;
        let ready = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/readyz", fixture.port))
            .bearer_auth(token(&fixture))
            .send()
            .await?;
        assert_eq!(ready.status(), 200);
        stop(child).await?;
        assert!(std::net::TcpListener::bind(("127.0.0.1", fixture.port)).is_ok());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn serve_unclassified_legacy_is_read_only_and_keeps_occupancy() -> TestResult {
    let fixture = Startup::new();
    configure(&fixture)?;
    let data = fixture.directory.path().join("data");
    std::fs::create_dir(&data)?;
    let path = data.join("shaula.db");
    let store = shaula_store::Store::open(&path).await?;
    store.migrate().await?;
    let db = Database::connect(format!("sqlite://{}?mode=rw", path.display())).await?;
    db.execute_unprepared("INSERT INTO fleets(key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at) VALUES ('legacy','incarnation',1,0,1,0,'Pending',0,1,1)").await?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite, "INSERT INTO runner_generations(id,fleet_key,runner_name,generation_name,fleet_revision,template_profile_key,template_revision,template_artifact_digest,attestation_id,inputs_digest,state,workspace_path,created_at,updated_at) VALUES (?,'legacy','runner','generation',1,'template',1,'sha256:legacy','activation','inputs','Creating',?,1,1)", vec![id.clone().into(), data.join("runners").join(&id).to_str().ok_or("path")?.into()])).await?;
    let child = launch(&fixture).await?;
    let client = reqwest::Client::builder().no_proxy().build()?;
    let origin = format!("http://127.0.0.1:{}", fixture.port);
    assert_eq!(
        client
            .get(format!("{origin}/readyz"))
            .bearer_auth(token(&fixture))
            .send()
            .await?
            .status(),
        503
    );
    let token = provider::sign(
        provider::claims(&fixture.provider.issuer, "api", "ops", provider::SCOPES),
        "at+jwt",
        "test-key",
    );
    assert_eq!(
        client
            .get(format!("{origin}/api/v1/generations"))
            .bearer_auth(&token)
            .send()
            .await?
            .status(),
        200
    );
    let denied = client
        .delete(format!("{origin}/api/v1/fleets/legacy"))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(denied.status(), 503);
    assert!(denied.text().await?.contains("MigrationRequired"));
    let row = db.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT state, (SELECT COUNT(*) FROM lifecycle_workers) AS workers FROM runner_generations")) .await?.ok_or("generation missing")?;
    assert_eq!(row.try_get::<String>("", "state")?, "Creating");
    assert_eq!(row.try_get::<i64>("", "workers")?, 0);
    stop(child).await?;
    let plan = fixture.directory.path().join("migration.json");
    let maintenance = |action: &str, path: &std::path::Path| {
        support::bare_command()
            .args(["maintenance", "lifecycle-state", action, "--config"])
            .arg(fixture.directory.path().join("bootstrap.json"))
            .arg("--plan")
            .arg(path)
            .output()
    };
    assert!(maintenance("inspect", &plan)?.status.success());
    db.execute_unprepared("UPDATE runner_generations SET github_runner_id = 7")
        .await?;
    assert!(
        !maintenance("apply", &plan)?.status.success(),
        "CI identity change without timestamp change invalidates the plan"
    );
    let fresh = fixture.directory.path().join("migration-current.json");
    assert!(maintenance("inspect", &fresh)?.status.success());
    for _ in 0..2 {
        assert!(
            maintenance("apply", &fresh)?.status.success(),
            "per-generation migration receipts replay"
        );
    }
    let row = db.query_one(Statement::from_string(DbBackend::Sqlite, "SELECT state, (SELECT activated FROM lifecycle_deployment) AS active, (SELECT COUNT(*) FROM generation_http_state) AS states FROM runner_generations")).await?.ok_or("missing")?;
    assert_eq!(row.try_get::<String>("", "state")?, "Quarantined");
    assert!(row.try_get::<bool>("", "active")?);
    assert_eq!(
        row.try_get::<i64>("", "states")?,
        0,
        "no fabricated legacy state"
    );
    Ok(())
}
