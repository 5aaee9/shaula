use super::forgejo_pool_support::{Fixture, TestResult};
use shaula_core::{
    jobs::{
        ForgejoJobsStore, ForgejoTaskConclusion, ForgejoTaskResult, JobsQuery, JobsReadPort,
        ObservedStatus,
    },
    ports::forgejo::ForgejoJob,
    registry::{ControlPlaneStore, FleetRuntimeGuard},
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

async fn fixture() -> TestResult<Fixture> {
    let fixture = Fixture::new(0, 2).await?;
    let guard = FleetRuntimeGuard::from(
        &fixture
            .store
            .fleet_get("fleet")
            .await?
            .ok_or("missing head")?,
    );
    fixture
        .store
        .forgejo_jobs_snapshot(
            "fleet",
            &guard,
            Some(&[ForgejoJob {
                id: 1,
                handle: "handle".into(),
                attempt: 1,
                status: "running".into(),
                runs_on: vec!["linux".into()],
                task_id: 42,
                run_id: 2,
                repo_id: 1,
                name: "build".into(),
            }]),
            1,
        )
        .await?;
    fixture
        .forgejo
        .history
        .lock()
        .map_err(|_| "history poisoned")?
        .push(ForgejoTaskResult {
            repository_id: 1,
            task_id: 42,
            conclusion: ForgejoTaskConclusion::Failure,
            owner: "owner".into(),
            repository: "repo".into(),
            run_number: 2,
            run_url: "https://forgejo.test/owner/repo/actions/runs/2".into(),
            workflow: "build.yml".into(),
        });
    Ok(fixture)
}

#[tokio::test]
async fn independent_forgejo_job_worker_publishes_only_exact_task_results() -> TestResult {
    let fixture = fixture().await?;
    fixture.supervisor().await?.enrich_jobs().await?;
    assert_eq!(fixture.forgejo.history_reads.load(Ordering::SeqCst), 1);
    let jobs = fixture
        .store
        .store()
        .list_jobs(JobsQuery::default())
        .await?;
    assert_eq!(jobs.items[0].observed_status, ObservedStatus::Completed);
    assert_eq!(jobs.items[0].reported_result.as_deref(), Some("failure"));
    assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.runtime.destroys.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn slow_history_does_not_hold_the_runner_tick_or_its_effect_gate() -> TestResult {
    let fixture = fixture().await?;
    fixture.forgejo.hold_history.store(true, Ordering::SeqCst);
    let supervisor = Arc::new(fixture.supervisor().await?);
    let worker = supervisor.clone();
    let task = tokio::spawn(async move { worker.enrich_jobs().await });
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture.forgejo.history_started.notified(),
    )
    .await?;
    let tick = tokio::time::timeout(Duration::from_secs(2), supervisor.tick()).await??;
    assert!(!tick.stale_demand);
    fixture.forgejo.history_release.notify_one();
    task.await??;
    Ok(())
}
