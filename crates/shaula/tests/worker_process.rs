//! Requires an explicitly delegated cgroup and pinned real Terraform. These
//! tests exercise the packaged binary's job role, not a fake Runtime adapter.
#![cfg(target_os = "linux")]
#[path = "worker_process/faults.rs"]
mod faults;
#[path = "worker_process/fixture.rs"]
mod fixture;
#[path = "worker_process/pre_exec.rs"]
mod pre_exec;
use fixture::*;
use shaula_core::{ports::*, registry::LifecycleStore, worker::Executor};
use std::time::Duration;

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn real_worker_create_wait_destroy_uses_sqlite_http_and_one_process() -> TestResult {
    let fixture = Fixture::new().await?;
    let result = fixture.create().await?;
    let before = fixture.backend.recovery_records().await?;
    let identity = before
        .first()
        .ok_or("worker missing")?
        .identity()?
        .ok_or("identity missing")?;
    assert_eq!(
        fixture.executor.observe(&identity).await,
        shaula_core::worker::ProcessObservation::Live
    );
    assert!(!fixture.workspace.join("terraform.tfstate").exists());
    let state = fixture.state().await?;
    assert!(!state.managed_empty());
    assert_eq!(state.lineage(), result.state_lineage);
    assert!(
        fixture.workers.create(fixture.request()).await.is_err(),
        "Create cannot be offered twice"
    );
    fixture.destroy(&result).await?;
    let state = fixture.state().await?;
    assert!(state.managed_empty());
    assert!(state.serial() > result.state_serial as i64);
    fixture
        .control_faults
        .lost_receipts
        .store(2, std::sync::atomic::Ordering::Release);
    fixture
        .control
        .generation_advance(
            &fixture.id.to_string(),
            shaula_core::lifecycle::GenerationState::Destroyed,
            10,
        )
        .await?;
    for _ in 0..30 {
        if fixture.executor.observe(&identity).await
            == shaula_core::worker::ProcessObservation::Exited
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        fixture
            .control_faults
            .lost_receipts
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
    assert_eq!(
        fixture.executor.observe(&identity).await,
        shaula_core::worker::ProcessObservation::Exited
    );
    let (stop, watch) = tokio::sync::watch::channel(false);
    fixture
        .reaper_failures
        .store(1, std::sync::atomic::Ordering::Release);
    let workers = fixture.workers.clone();
    let reaper = tokio::spawn(async move { workers.supervise(watch).await });
    for _ in 0..50 {
        if !fixture.workspace.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !fixture.workspace.exists(),
        "sealed ordinary workspace is reaped after a transient cleanup error"
    );
    stop.send(true)?;
    reaper.await??;
    fixture.close().await
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn fenced_worker_replacement_restores_missing_workspace_without_create() -> TestResult {
    let fixture = Fixture::new().await?;
    let result = fixture.create().await?;
    let before = fixture.backend.recovery_records().await?;
    let identity = before
        .first()
        .ok_or("worker missing")?
        .identity()?
        .ok_or("identity missing")?;
    assert_eq!(
        fixture.executor.stop_and_fence(&identity).await,
        shaula_core::worker::FenceOutcome::Fenced
    );
    // This is an isolated TempDir fixture; retain proof/state in SQLite and
    // deliberately remove only its disposable materialized workspace.
    assert!(fixture.workspace.starts_with(fixture.root.path()));
    std::fs::remove_dir_all(&fixture.workspace)?;
    fixture.destroy(&result).await?;
    let after = fixture.backend.recovery_records().await?;
    assert_eq!(after.first().ok_or("replacement missing")?.worker_epoch, 2);
    assert_eq!(fixture.create_count().await?, 1);
    assert!(fixture.state().await?.managed_empty());
    fixture.close().await
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn killed_worker_with_detached_provider_descendant_is_not_a_fence() -> TestResult {
    let fixture = Fixture::with_orphan(true).await?;
    let _ = fixture.create().await?;
    let identity = fixture
        .backend
        .recovery_records()
        .await?
        .pop()
        .ok_or("worker absent")?
        .identity()?
        .ok_or("identity absent")?;
    let pid = std::fs::read_to_string(fixture.workspace.join("orphan.pid"))?;
    let pid: u32 = pid.trim().parse()?;
    assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(std::process::Command::new("kill")
        .args(["-KILL", &identity.process_id.to_string()])
        .status()?
        .success());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        fixture.executor.observe(&identity).await,
        shaula_core::worker::ProcessObservation::Unknown
    );
    assert_eq!(
        fixture.executor.stop_and_fence(&identity).await,
        shaula_core::worker::FenceOutcome::Fenced
    );
    assert_eq!(
        fixture.executor.observe(&identity).await,
        shaula_core::worker::ProcessObservation::Exited
    );
    assert!(
        !fixture.state().await?.managed_empty(),
        "process death does not release provider resources"
    );
    fixture.close().await
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP and SHAULA_TEST_TERRAFORM"]
async fn failed_final_state_post_retains_emergency_evidence_and_blocks_reapply() -> TestResult {
    use std::sync::atomic::Ordering;
    let fixture = Fixture::new().await?;
    let result = fixture.create().await?;
    fixture
        .faults
        .reject_empty_writes
        .store(true, Ordering::Release);
    assert!(fixture.destroy(&result).await.is_err());
    let rejected = fixture.faults.rejected.load(Ordering::Acquire);
    assert!(
        rejected > 0,
        "actual Terraform attempted its final state POST"
    );
    let path = fixture.workspace.join("errored.tfstate");
    let evidence = std::fs::read(&path)?;
    let emergency = shaula_core::state_backend::StateDocument::parse(evidence.clone())?;
    assert!(
        emergency.managed_empty(),
        "Destroy completed outside the database"
    );
    assert!(
        !fixture.state().await?.managed_empty(),
        "old database state stays authoritative"
    );
    assert!(
        fixture
            .control
            .generation_advance(
                &fixture.id.to_string(),
                shaula_core::lifecycle::GenerationState::Destroyed,
                10,
            )
            .await
            .is_err(),
        "no terminal/capacity release after failed upload"
    );
    fixture
        .faults
        .reject_empty_writes
        .store(false, Ordering::Release);
    assert!(
        fixture.destroy(&result).await.is_err(),
        "restored transport cannot silently choose between conflicting states"
    );
    assert_eq!(
        std::fs::read(path)?,
        evidence,
        "emergency evidence survives retry"
    );
    assert_eq!(fixture.faults.rejected.load(Ordering::Acquire), rejected);
    assert_eq!(fixture.create_count().await?, 1);
    fixture.close().await
}
