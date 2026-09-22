//! Jobs history is not an authority for Runner lifecycle effects.
use super::forgejo_pool_support::{Fixture, TestResult};
use sea_orm::ConnectionTrait;
use shaula_core::{
    error::ReasonCode,
    jobs::{JobsQuery, JobsReadPort},
    lifecycle::GenerationState as G,
    ports::forgejo::ForgejoJob,
    registry::ControlPlaneStore,
};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn jobs_write_failure_does_not_block_completed_runner_reclaim() -> TestResult {
    let fixture = Fixture::new(0, 1).await?;
    fixture.forgejo.waiting.store(1, Ordering::SeqCst);
    assert_eq!(fixture.supervisor().await?.tick().await?.created, 1);
    fixture.declare("idle")?;
    fixture.supervisor().await?.tick().await?;
    fixture.declare("active")?;
    fixture.forgejo.waiting.store(0, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    assert_eq!(fixture.generation().await?.state, G::Busy);
    fixture
        .forgejo
        .runners
        .lock()
        .map_err(|_| "poisoned")?
        .clear();

    // Only the optional Jobs projection fails; lifecycle persistence stays healthy.
    fixture
        .store
        .store()
        .connection()
        .execute_unprepared(
            "CREATE TRIGGER reject_job_poll BEFORE INSERT ON forgejo_job_polls
         BEGIN SELECT RAISE(FAIL, 'injected Jobs failure'); END",
        )
        .await?;
    let report = fixture.supervisor().await?.tick().await?;
    assert!(!report.stale_demand);
    assert!(
        report.reason.is_some(),
        "history failure remains observable"
    );
    assert_eq!(report.destroyed, 1);
    assert_eq!(fixture.generation().await?.state, G::Destroyed);
    assert_eq!(fixture.store.generations_occupancy("fleet").await?, 0);
    assert_eq!(fixture.runtime.destroys.load(Ordering::SeqCst), 1);
    Ok(())
}

fn job() -> ForgejoJob {
    ForgejoJob {
        id: 1,
        repo_id: 1,
        attempt: 1,
        run_id: 1,
        task_id: 0,
        handle: "opaque".into(),
        status: "waiting".into(),
        runs_on: vec!["linux".into()],
        name: "test".into(),
    }
}

#[tokio::test]
async fn metadata_rejection_keeps_history_stale_without_discarding_valid_demand() -> TestResult {
    let fixture = Fixture::new(0, 2).await?;
    let at = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )? - 1_000;
    fixture.clock.0.store(at, Ordering::SeqCst);
    fixture.forgejo.waiting.store(1, Ordering::SeqCst);
    assert_eq!(fixture.supervisor().await?.tick().await?.created, 1);
    let previous = fixture
        .store
        .store()
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .remove(0);
    assert_eq!(previous.freshness, "fresh");
    fixture.declare("idle")?;

    *fixture.forgejo.jobs.lock().map_err(|_| "poisoned")? = Some(vec![
        ForgejoJob {
            name: "x".repeat(1025),
            ..job()
        },
        ForgejoJob { id: 2, ..job() },
    ]);
    fixture.clock.0.store(at + 1, Ordering::SeqCst);
    let report = fixture.supervisor().await?.tick().await?;
    assert!(!report.stale_demand);
    assert_eq!(
        (report.waiting_jobs, report.target, report.created),
        (2, 2, 1)
    );
    assert_eq!(fixture.store.demand_get("fleet").await?, Some(2));
    let history = fixture
        .store
        .store()
        .get_job(&previous.id)
        .await?
        .ok_or("missing history")?;
    assert_eq!(history.job.freshness, "stale");
    assert_eq!(history.job.updated_at, previous.updated_at);
    assert_eq!(
        history.job.metadata.job_display_name.as_deref(),
        Some("test")
    );
    assert_eq!(history.forgejo_observations.len(), 1);
    Ok(())
}

#[tokio::test]
async fn invalid_snapshot_identities_never_drive_capacity_during_a_history_outage() -> TestResult {
    for jobs in [
        vec![ForgejoJob { id: 0, ..job() }],
        vec![ForgejoJob {
            repo_id: 0,
            ..job()
        }],
        vec![job(), job()],
        vec![job(); 10_001],
    ] {
        let fixture = Fixture::new(0, 2).await?;
        *fixture.forgejo.jobs.lock().map_err(|_| "poisoned")? = Some(jobs);
        fixture
            .store
            .store()
            .connection()
            .execute_unprepared(
                "CREATE TRIGGER reject_job_poll BEFORE INSERT ON forgejo_job_polls
             BEGIN SELECT RAISE(FAIL, 'injected Jobs failure'); END",
            )
            .await?;
        let error = fixture
            .supervisor()
            .await?
            .tick()
            .await
            .err()
            .ok_or("invalid identities admitted")?;
        assert_eq!(error.code, ReasonCode::AccessVerificationFailed);
        assert_eq!(fixture.store.demand_get("fleet").await?, None);
        assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.store.generations_occupancy("fleet").await?, 0);
    }
    Ok(())
}

#[tokio::test]
async fn poll_and_history_failure_preserve_demand_but_allow_inventory_readiness() -> TestResult {
    let fixture = Fixture::new(0, 2).await?;
    fixture.forgejo.waiting.store(1, Ordering::SeqCst);
    fixture.supervisor().await?.tick().await?;
    let previous = fixture
        .store
        .store()
        .demand_get("fleet")
        .await?
        .ok_or("missing demand")?;
    fixture.declare("idle")?;
    fixture.forgejo.fail_jobs.store(true, Ordering::SeqCst);
    fixture.clock.0.store(2_000, Ordering::SeqCst);
    fixture
        .store
        .store()
        .connection()
        .execute_unprepared(
            "CREATE TRIGGER reject_job_poll BEFORE INSERT ON forgejo_job_polls
         BEGIN SELECT RAISE(FAIL, 'injected Jobs failure'); END",
        )
        .await?;
    let report = fixture.supervisor().await?.tick().await?;
    assert!(report.stale_demand);
    assert_eq!((report.created, report.destroyed), (0, 0));
    assert_eq!(fixture.generation().await?.state, G::Idle);
    let after = fixture
        .store
        .store()
        .demand_get("fleet")
        .await?
        .ok_or("missing demand")?;
    assert_eq!(
        (after.total_assigned_jobs, after.updated_at),
        (previous.total_assigned_jobs, previous.updated_at)
    );
    Ok(())
}
