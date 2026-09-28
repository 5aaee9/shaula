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
    // Startup hashes the unoptimized debug executable for the job role,
    // which alone can approach 15 seconds on a shared CI runner.
    for _ in 0..600 {
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

/// LW-11 through the real daemon: a SIGKILL after `launch_pending` committed
/// and the attempt's process was spawned, but before its identity became
/// durable, leaves exactly this residue. The restarted `serve` must classify it
/// from the attempt's containment (killing the unenveloped job and any stray
/// member), record a confirmed fence and rotate instead of quarantining.
#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn serve_restart_fences_a_pre_registration_crash_by_exact_containment() -> TestResult {
    use shaula_core::{
        lifecycle::GenerationState as G,
        registry::{FleetRuntimeGuard, GenerationRecord, LifecycleStore},
        worker::{ControlAccess, Executor, ProcessObservation},
    };
    use shaula_store::{
        http_state::{SqliteStateBackend, WorkerAdmissions},
        registry_impl::SqliteControlPlane,
    };
    use std::sync::Arc;

    let fixture = Startup::new();
    configure(&fixture)?;
    stop(launch(&fixture).await?).await?;
    let data = fixture.directory.path().join("data");
    let path = data.join("shaula.db");
    let db = Database::connect(format!("sqlite://{}?mode=rw", path.display())).await?;
    db.execute_unprepared("INSERT INTO fleets(key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at) VALUES ('crashed','incarnation',1,0,1,0,'Pending',0,1,1)").await?;
    let store = shaula_store::Store::open(&path).await?;
    let admissions = Arc::new(WorkerAdmissions::new(4, 1)?);
    let control = SqliteControlPlane::new(store.clone(), data.join("template-artifacts"))
        .with_worker_admissions(admissions.clone());
    let id = uuid::Uuid::new_v4();
    let workspace = data.join("runners").join(id.to_string());
    std::fs::create_dir_all(&workspace)?;
    let record = GenerationRecord {
        id: id.to_string(),
        fleet_key: "crashed".into(),
        runner_name: "runner".into(),
        generation_name: format!("s{}", id.simple()),
        fleet_revision: 1,
        template_profile_key: "template".into(),
        pool_member_key: None,
        template_revision: 1,
        template_artifact_digest: "sha256:fixture".into(),
        attestation_id: "activation".into(),
        inputs_digest: "fixture".into(),
        state: G::CreatePending,
        github_runner_id: None,
        workspace_path: workspace.to_str().ok_or("workspace encoding")?.into(),
        created_at: 1,
        updated_at: 1,
    };
    let guard = FleetRuntimeGuard {
        incarnation: "incarnation".into(),
        desired_revision: 1,
        mutation_fence: 1,
    };
    assert!(control.generation_insert_guarded(record, &guard).await?);
    control
        .generation_advance(&id.to_string(), G::Creating, 2)
        .await?;
    let admission = admissions.take(id)?;
    let access = ControlAccess {
        claim: admission.claim.clone(),
        capability: admission.control.clone(),
    };
    // The crashed daemon's last durable step, then its unregistered spawn.
    SqliteStateBackend::new(store.clone())
        .worker_launch_pending(&access)
        .await?;
    let crashed = shaula_executor::ExecExecutor::new(
        std::path::PathBuf::from(env!("CARGO_BIN_EXE_shaula")),
        std::path::PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?),
    )?;
    let identity = crashed.launch(&admission.claim).await?;
    let group = std::path::PathBuf::from(&identity.containment);
    let mut stray = std::process::Command::new("sleep").arg("300").spawn()?;
    std::fs::write(group.join("cgroup.procs"), stray.id().to_string())?;
    assert_eq!(crashed.observe(&identity).await, ProcessObservation::Live);
    drop(store);

    let child = launch(&fixture).await?;
    assert!(!group.exists(), "the attempt's exact group was fenced");
    assert!(stray.try_wait()?.is_some(), "stray member was killed");
    assert_ne!(
        crashed.observe(&identity).await,
        ProcessObservation::Live,
        "the unenveloped job was killed"
    );
    let fence = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT outcome,confirmed_fenced FROM lifecycle_fences WHERE worker_attempt=?",
            vec![admission.claim.worker_attempt.to_string().into()],
        ))
        .await?
        .ok_or("fence record missing")?;
    assert_eq!(fence.try_get::<String>("", "outcome")?, "fenced");
    let row = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT g.state, w.phase FROM runner_generations g JOIN lifecycle_workers w ON w.generation_id=g.id WHERE g.id=?",
            vec![id.to_string().into()],
        ))
        .await?
        .ok_or("generation missing")?;
    assert_ne!(row.try_get::<String>("", "state")?, "Quarantined");
    assert_ne!(row.try_get::<String>("", "phase")?, "quarantined");
    stop(child).await?;
    Ok(())
}
