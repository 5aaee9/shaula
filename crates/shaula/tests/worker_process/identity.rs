//! LW-14: PID reuse, host tuple and containment loss. A fence is proved only
//! by the recorded host/boot and an empty kernel-owned containment; a PID,
//! its absence or a vanished directory never fabricates one.
use shaula_core::{
    state_backend::StateClaim,
    worker::{Executor, FenceOutcome, ProcessIdentity, ProcessObservation},
};
use shaula_executor::ExecExecutor;
use std::path::PathBuf;
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn executor() -> TestResult<ExecExecutor> {
    Ok(ExecExecutor::new(
        PathBuf::from(env!("CARGO_BIN_EXE_shaula")),
        PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?),
    )?)
}

async fn launched(executor: &ExecExecutor) -> TestResult<ProcessIdentity> {
    Ok(executor
        .launch(&StateClaim {
            generation_id: Uuid::new_v4(),
            worker_epoch: 1,
            worker_attempt: Uuid::new_v4(),
        })
        .await?)
}

fn alive(child: &mut std::process::Child) -> TestResult<bool> {
    Ok(child.try_wait()?.is_none())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn reused_pid_is_not_liveness_and_is_never_signalled() -> TestResult {
    let executor = executor()?;
    let identity = launched(&executor).await?;
    // An unrelated process now owns the recorded PID number. Start times
    // have clock-tick granularity, so make sure the two cannot coincide.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut unrelated = std::process::Command::new("sleep").arg("300").spawn()?;
    let reused = ProcessIdentity {
        process_id: unrelated.id(),
        ..identity.clone()
    };
    assert_ne!(
        executor.observe(&reused).await,
        ProcessObservation::Live,
        "a PID whose start time differs is not the recorded worker"
    );
    assert_eq!(executor.stop_and_fence(&reused).await, FenceOutcome::Fenced);
    assert!(alive(&mut unrelated)?, "fencing never signals by PID");
    assert_eq!(
        executor.observe(&identity).await,
        ProcessObservation::Exited
    );
    executor.release_fenced(&identity).await?;
    unrelated.kill()?;
    unrelated.wait()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn foreign_host_identity_is_never_fenced_locally() -> TestResult {
    let executor = executor()?;
    let identity = launched(&executor).await?;
    let (_, boot) = identity.host_boot.split_once(':').ok_or("host tuple")?;
    let foreign = ProcessIdentity {
        host_boot: format!("{}:{boot}", Uuid::new_v4().simple()),
        ..identity.clone()
    };
    assert_eq!(
        executor.observe(&foreign).await,
        ProcessObservation::Unknown
    );
    assert_eq!(
        executor.stop_and_fence(&foreign).await,
        FenceOutcome::Unknown
    );
    assert!(executor.release_fenced(&foreign).await.is_err());
    // The local tree was not touched on behalf of another host.
    assert_eq!(executor.observe(&identity).await, ProcessObservation::Live);
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    executor.release_fenced(&identity).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn previous_boot_on_the_same_host_is_fenced_by_reboot() -> TestResult {
    let executor = executor()?;
    let identity = launched(&executor).await?;
    let (machine, _) = identity.host_boot.split_once(':').ok_or("host tuple")?;
    let root = PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?);
    let old_boot = ProcessIdentity {
        host_boot: format!("{machine}:{}", Uuid::new_v4()),
        containment: root
            .join(format!("shaula-{}", Uuid::new_v4()))
            .to_string_lossy()
            .into_owned(),
        ..identity.clone()
    };
    assert_eq!(
        executor.observe(&old_boot).await,
        ProcessObservation::Exited
    );
    assert_eq!(
        executor.stop_and_fence(&old_boot).await,
        FenceOutcome::Fenced
    );
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    executor.release_fenced(&identity).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn containment_outside_the_delegated_root_is_rejected() -> TestResult {
    let executor = executor()?;
    let identity = launched(&executor).await?;
    let root = PathBuf::from(std::env::var("SHAULA_TEST_CGROUP")?);
    let name = PathBuf::from(&identity.containment);
    let name = name.file_name().ok_or("containment name")?;
    for containment in [
        root.parent().ok_or("root parent")?.join(name),
        root.join("not-shaula"),
        root.join(name).join("nested"),
    ] {
        let tampered = ProcessIdentity {
            containment: containment.to_string_lossy().into_owned(),
            ..identity.clone()
        };
        assert_eq!(
            executor.observe(&tampered).await,
            ProcessObservation::Unknown
        );
        assert_eq!(
            executor.stop_and_fence(&tampered).await,
            FenceOutcome::Unknown
        );
    }
    assert_eq!(executor.observe(&identity).await, ProcessObservation::Live);
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    executor.release_fenced(&identity).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SHAULA_TEST_CGROUP"]
async fn vanished_containment_on_the_same_boot_is_not_a_fence() -> TestResult {
    let executor = executor()?;
    let identity = launched(&executor).await?;
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Fenced
    );
    executor.release_fenced(&identity).await?;
    // Once the kernel evidence is gone, only a durable receipt can speak for
    // it. Absence is never re-certified as a new fence.
    assert_eq!(
        executor.observe(&identity).await,
        ProcessObservation::Unknown
    );
    assert_eq!(
        executor.stop_and_fence(&identity).await,
        FenceOutcome::Unknown
    );
    Ok(())
}
