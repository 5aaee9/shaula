use super::forgejo_tests::{fixture, job};
use sea_orm::ConnectionTrait;
use shaula_core::jobs::{
    AssociationStatus, ForgejoJobsStore, ForgejoTaskConclusion, ForgejoTaskResult, JobsQuery,
    JobsReadPort, ObservedStatus,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[tokio::test]
async fn pre_enrichment_records_and_observations_remain_readable() -> TestResult {
    let (directory, store, guard) = fixture().await?;
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[job()]), 1000)
        .await?;
    store.store().connection().execute_unprepared(
        "UPDATE forgejo_workflow_jobs SET summary_json=json_remove(summary_json,'$.forgejo.result','$.forgejo.enrichment_attempted_at');
         UPDATE forgejo_job_observations SET data_json=json_remove(data_json,'$.source');",
    ).await?;
    let reopened = crate::Store::open(&directory.path().join("jobs.db")).await?;
    reopened.migrate().await?;
    let page = reopened.list_jobs(JobsQuery::default()).await?;
    assert_eq!(page.items.len(), 1);
    let detail = reopened
        .get_job(&page.items[0].id)
        .await?
        .ok_or("missing job")?;
    assert!(detail
        .job
        .forgejo
        .ok_or("missing metadata")?
        .result
        .is_none());
    assert_eq!(
        detail.forgejo_observations[0].source,
        shaula_core::jobs::ForgejoObservationSource::RunnerSnapshot
    );
    assert_eq!(detail.job.association_status, AssociationStatus::Unverified);
    Ok(())
}

fn proof() -> ForgejoTaskResult {
    ForgejoTaskResult {
        repository_id: 9,
        task_id: 42,
        conclusion: ForgejoTaskConclusion::Success,
        owner: "owner".into(),
        repository: "repo".into(),
        run_number: 8,
        run_url: "https://forgejo.test/owner/repo/actions/runs/8".into(),
        workflow: "test.yml".into(),
    }
}

#[tokio::test]
async fn job_results_recheck_fleet_scope_task_identity_and_url_before_publication() -> TestResult {
    let (_directory, store, guard) = fixture().await?;
    let task = shaula_core::ports::forgejo::ForgejoJob {
        task_id: 42,
        status: "running".into(),
        ..job()
    };
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(std::slice::from_ref(&task)), 1000)
        .await?;
    let pending = store
        .forgejo_jobs_pending_results("forgejo", &guard, 2000)
        .await?;
    let lookup = &pending[0];
    for result in [
        ForgejoTaskResult {
            repository_id: 10,
            ..proof()
        },
        ForgejoTaskResult {
            task_id: 43,
            ..proof()
        },
    ] {
        assert!(
            !store
                .forgejo_job_result("forgejo", &guard, lookup, &result, 3000)
                .await?
        );
    }
    let malicious = ForgejoTaskResult {
        run_url: "https://other.test/run".into(),
        ..proof()
    };
    assert!(store
        .forgejo_job_result("forgejo", &guard, lookup, &malicious, 3000)
        .await
        .is_err());
    let mut stale = guard.clone();
    stale.mutation_fence += 1;
    assert!(
        !store
            .forgejo_job_result("forgejo", &stale, lookup, &proof(), 3000)
            .await?
    );
    store
        .forgejo_jobs_snapshot(
            "forgejo",
            &guard,
            Some(&[shaula_core::ports::forgejo::ForgejoJob {
                task_id: 43,
                ..task
            }]),
            3000,
        )
        .await?;
    assert!(
        !store
            .forgejo_job_result("forgejo", &guard, lookup, &proof(), 4000)
            .await?
    );
    let record = store
        .store()
        .get_job(&lookup.record_id)
        .await?
        .ok_or("missing record")?;
    assert_eq!(record.job.reported_result, None);
    assert_eq!(record.job.observed_status, ObservedStatus::Running);
    Ok(())
}

#[tokio::test]
async fn task_results_commit_atomically_and_replays_do_not_refresh_evidence() -> TestResult {
    let (_directory, store, guard) = fixture().await?;
    store
        .forgejo_jobs_snapshot(
            "forgejo",
            &guard,
            Some(&[shaula_core::ports::forgejo::ForgejoJob {
                task_id: 42,
                ..job()
            }]),
            1000,
        )
        .await?;
    let pending = store
        .forgejo_jobs_pending_results("forgejo", &guard, 2000)
        .await?;
    let lookup = &pending[0];
    store.store().connection().execute_unprepared("CREATE TRIGGER fail_result BEFORE INSERT ON forgejo_job_observations
        WHEN json_extract(NEW.data_json,'$.source')='task_history' BEGIN SELECT RAISE(ABORT,'injected'); END").await?;
    assert!(store
        .forgejo_job_result("forgejo", &guard, lookup, &proof(), 3000)
        .await
        .is_err());
    assert_eq!(
        store
            .store()
            .get_job(&lookup.record_id)
            .await?
            .ok_or("missing job")?
            .job
            .reported_result,
        None
    );
    store
        .store()
        .connection()
        .execute_unprepared("DROP TRIGGER fail_result")
        .await?;
    assert!(
        store
            .forgejo_job_result("forgejo", &guard, lookup, &proof(), 3000)
            .await?
    );
    let before = store
        .store()
        .get_job(&lookup.record_id)
        .await?
        .ok_or("missing job")?;
    assert!(
        store
            .forgejo_job_result("forgejo", &guard, lookup, &proof(), 4000)
            .await?
    );
    assert_eq!(
        store.store().get_job(&lookup.record_id).await?,
        Some(before)
    );
    Ok(())
}

#[tokio::test]
async fn history_budget_survives_restart_and_never_looks_up_unassigned_jobs() -> TestResult {
    let (directory, store, guard) = fixture().await?;
    let mut jobs = vec![job()];
    jobs.extend((1..=50).map(|id| shaula_core::ports::forgejo::ForgejoJob {
        id: id + 10,
        repo_id: id % 5 + 1,
        task_id: id + 100,
        ..job()
    }));
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&jobs), 1000)
        .await?;
    let first = store
        .forgejo_jobs_pending_results("forgejo", &guard, 2000)
        .await?;
    assert_eq!(first.len(), 20);
    assert!(first.iter().all(|lookup| lookup.task_id > 0));
    assert!(
        first
            .iter()
            .map(|lookup| lookup.repository_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            <= 4
    );
    let reopened = crate::registry_impl::SqliteControlPlane::new(
        crate::Store::open(&directory.path().join("jobs.db")).await?,
        directory.path().join("artifacts"),
    );
    assert!(reopened
        .forgejo_jobs_pending_results("forgejo", &guard, 31_999)
        .await?
        .is_empty());
    let second = reopened
        .forgejo_jobs_pending_results("forgejo", &guard, 32_000)
        .await?;
    assert!(!second.is_empty());
    assert!(second.iter().all(|lookup| !first.contains(lookup)));
    Ok(())
}

#[tokio::test]
async fn two_attempts_claiming_the_same_task_remain_unresolved() -> TestResult {
    let (_directory, store, guard) = fixture().await?;
    let jobs = [
        shaula_core::ports::forgejo::ForgejoJob {
            task_id: 42,
            ..job()
        },
        shaula_core::ports::forgejo::ForgejoJob {
            attempt: 2,
            task_id: 42,
            ..job()
        },
    ];
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&jobs), 1000)
        .await?;
    for lookup in store
        .forgejo_jobs_pending_results("forgejo", &guard, 2000)
        .await?
    {
        assert!(
            !store
                .forgejo_job_result("forgejo", &guard, &lookup, &proof(), 3000)
                .await?
        );
    }
    assert!(store
        .store()
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .iter()
        .all(|job| job.reported_result.is_none()));
    Ok(())
}

#[tokio::test]
async fn exact_task_result_survives_absence_late_snapshots_and_restart_without_runner_association(
) -> TestResult {
    let (directory, store, guard) = fixture().await?;
    let task = shaula_core::ports::forgejo::ForgejoJob {
        task_id: 42,
        status: "running".into(),
        ..job()
    };
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(std::slice::from_ref(&task)), 1000)
        .await?;
    let pending = store
        .forgejo_jobs_pending_results("forgejo", &guard, 2000)
        .await?;
    assert_eq!(pending.len(), 1);
    let result = ForgejoTaskResult {
        repository_id: 9,
        task_id: 42,
        conclusion: ForgejoTaskConclusion::Success,
        owner: "owner".into(),
        repository: "repo".into(),
        run_number: 8,
        run_url: "https://forgejo.test/owner/repo/actions/runs/8".into(),
        workflow: "test.yml".into(),
    };
    assert!(
        store
            .forgejo_job_result("forgejo", &guard, &pending[0], &result, 3000)
            .await?
    );
    // A poll started before the independent history reader can commit later.
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[]), 2500)
        .await?;
    assert_eq!(
        store
            .store()
            .get_job(&pending[0].record_id)
            .await?
            .ok_or("missing job")?
            .job
            .updated_at,
        3000
    );
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[task]), 4000)
        .await?;
    store
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[]), 5000)
        .await?;
    let reopened = crate::Store::open(&directory.path().join("jobs.db")).await?;
    let detail = reopened
        .get_job(&pending[0].record_id)
        .await?
        .ok_or("missing job")?;
    assert_eq!(detail.job.observed_status, ObservedStatus::Completed);
    assert_eq!(detail.job.reported_result.as_deref(), Some("success"));
    assert_eq!(detail.job.freshness, "confirmed");
    assert_eq!(detail.job.association_status, AssociationStatus::Unverified);
    assert!(detail.generations.is_empty());
    assert!(detail.job.github_run_url.is_none());
    assert_eq!(
        detail
            .job
            .forgejo
            .ok_or("missing Forgejo data")?
            .result
            .ok_or("missing result")?
            .task_id,
        "42"
    );
    assert!(store
        .forgejo_jobs_pending_results("forgejo", &guard, 40_000)
        .await?
        .is_empty());
    assert_eq!(
        reopened
            .list_jobs(JobsQuery {
                status: Some("completed".into()),
                ..Default::default()
            })
            .await?
            .items
            .len(),
        1
    );
    Ok(())
}
