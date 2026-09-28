//! LW-14 under a service manager: systemd removes a delegated unit's whole
//! cgroup subtree after control-group termination, so an ungraceful restart
//! finds no containment directory. Only a replaced delegated root (a new
//! kernel cgroup ID on the same boot) proves that old tree stopped.
use super::identity::{executor, launched, TestResult};
use shaula_core::worker::{Executor, FenceOutcome, ProcessIdentity, ProcessObservation};
use std::{os::unix::fs::MetadataExt, path::PathBuf, time::Duration};

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn launch_records_the_delegated_root_cgroup_id() -> TestResult {
    let executor = executor()?;
    let identity = launched(&executor).await?;
    let root = PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?);
    assert_eq!(identity.root_id, Some(std::fs::metadata(&root)?.ino()));
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    executor.release_fenced(&identity).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn replaced_root_on_the_same_boot_is_a_fence_but_legacy_absence_is_not() -> TestResult {
    let executor = executor()?;
    let identity = launched(&executor).await?;
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    executor.release_fenced(&identity).await?;
    let recorded = identity.root_id.ok_or("root id absent")?;
    // As recorded by a previous incarnation of this delegated root.
    let replaced = ProcessIdentity {
        root_id: Some(recorded.wrapping_add(1)),
        ..identity.clone()
    };
    assert_eq!(
        executor.observe(&replaced).await,
        ProcessObservation::Exited
    );
    assert_eq!(
        executor.stop_and_fence(&replaced).await,
        FenceOutcome::Fenced
    );
    // An identity recorded before root IDs existed keeps failing closed.
    let legacy = ProcessIdentity {
        root_id: None,
        ..identity
    };
    assert_eq!(executor.observe(&legacy).await, ProcessObservation::Unknown);
    assert_eq!(
        executor.stop_and_fence(&legacy).await,
        FenceOutcome::Unknown
    );
    Ok(())
}

/// Driven by `scripts/lifecycle-systemd-restart.sh` across two invocations
/// of the same transient unit; a no-op when run without its role.
#[tokio::test]
#[ignore = "driven by scripts/lifecycle-systemd-restart.sh"]
async fn systemd_restart_role() -> TestResult {
    let Ok(role) = std::env::var("SHAULA_RESTART_ROLE") else {
        eprintln!("systemd_restart_role: no role; run the restart script");
        return Ok(());
    };
    let file = PathBuf::from(std::env::var("SHAULA_RESTART_IDENTITY")?);
    let executor = executor()?;
    if role == "launch" {
        let identity = launched(&executor).await?;
        std::fs::write(&file, serde_json::to_vec(&identity)?)?;
        // Held until the unit is killed as an ungraceful daemon exit.
        tokio::time::sleep(Duration::from_secs(600)).await;
        return Err("launch role was not killed".into());
    }
    let identity: ProcessIdentity = serde_json::from_slice(&std::fs::read(&file)?)?;
    assert!(
        !PathBuf::from(&identity.containment).exists(),
        "systemd removed the old delegated subtree"
    );
    assert_eq!(
        executor.observe(&identity).await,
        ProcessObservation::Exited
    );
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    let legacy = ProcessIdentity {
        root_id: None,
        ..identity
    };
    assert_eq!(
        executor.stop_and_fence(&legacy).await,
        FenceOutcome::Unknown
    );
    println!("systemd restart: replaced root fenced; legacy identity stays Unknown");
    Ok(())
}
