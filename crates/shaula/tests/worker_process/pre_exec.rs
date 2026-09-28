//! LW-11: a daemon crash after launch but before durable registration leaves
//! a launch_pending attempt without identity. Restart must classify it from
//! the attempt's exact containment, not from its own empty child list.
use shaula_core::{
    state_backend::StateClaim,
    worker::{Executor, FenceOutcome, ProcessObservation},
};
use shaula_executor::ExecExecutor;
use std::path::{Path, PathBuf};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn executor() -> TestResult<ExecExecutor> {
    Ok(ExecExecutor::new(
        PathBuf::from(env!("CARGO_BIN_EXE_shaula")),
        PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?),
    )?)
}

fn claim() -> StateClaim {
    StateClaim {
        generation_id: Uuid::new_v4(),
        worker_epoch: 1,
        worker_attempt: Uuid::new_v4(),
    }
}

fn group(claim: &StateClaim) -> TestResult<PathBuf> {
    Ok(PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?)
        .join(format!("shaula-{}", claim.worker_attempt)))
}

fn members(path: &Path) -> TestResult<Vec<u32>> {
    Ok(std::fs::read_to_string(path.join("cgroup.procs"))?
        .lines()
        .map(str::parse)
        .collect::<Result<_, _>>()?)
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn unregistered_job_blocked_on_handoff_is_killed_by_exact_containment() -> TestResult {
    let crashed = executor()?;
    let claim = claim();
    // The crashed daemon launched but never registered or handed off.
    let identity = crashed.launch(&claim).await?;
    let path = group(&claim)?;
    assert_eq!(members(&path)?, vec![identity.process_id]);
    assert_eq!(crashed.observe(&identity).await, ProcessObservation::Live);

    let restarted = executor()?;
    assert_eq!(
        restarted.fence_unregistered(&claim).await,
        FenceOutcome::Fenced
    );
    assert!(!path.exists(), "unnamed empty group is not evidence");
    assert_eq!(
        crashed.observe(&identity).await,
        ProcessObservation::Unknown
    );
    // Repeating recovery after a crash during classification stays fenced.
    assert_eq!(
        restarted.fence_unregistered(&claim).await,
        FenceOutcome::Fenced
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn unregistered_fence_is_exact_to_its_attempt() -> TestResult {
    let executor = executor()?;
    let live = claim();
    let identity = executor.launch(&live).await?;
    // Same Generation, another attempt: never touches the live group.
    let other = StateClaim {
        worker_attempt: Uuid::new_v4(),
        ..live.clone()
    };
    assert_eq!(
        executor.fence_unregistered(&other).await,
        FenceOutcome::Fenced
    );
    assert_eq!(executor.observe(&identity).await, ProcessObservation::Live);
    assert_eq!(members(&group(&live)?)?, vec![identity.process_id]);
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    executor.release_fenced(&identity).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn unregistered_descendant_outside_the_job_is_still_fenced() -> TestResult {
    let crashed = executor()?;
    let claim = claim();
    let identity = crashed.launch(&claim).await?;
    let path = group(&claim)?;
    // A process that joined the group by other means (e.g. a detached
    // descendant) is covered by the same kernel-owned set.
    let mut stray = std::process::Command::new("sleep").arg("300").spawn()?;
    std::fs::write(path.join("cgroup.procs"), stray.id().to_string())?;
    assert_eq!(members(&path)?.len(), 2);
    let restarted = executor()?;
    assert_eq!(
        restarted.fence_unregistered(&claim).await,
        FenceOutcome::Fenced
    );
    assert!(stray.try_wait()?.is_some(), "stray member was killed");
    assert_eq!(
        crashed.observe(&identity).await,
        ProcessObservation::Unknown
    );
    Ok(())
}
