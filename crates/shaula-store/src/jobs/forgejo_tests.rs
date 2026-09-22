use crate::{registry_impl::SqliteControlPlane, Store};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::{
    jobs::{AssociationStatus, ForgejoJobsStore, JobsQuery, JobsReadPort, ObservedStatus},
    ports::forgejo::ForgejoJob,
    registry::{ControlPlaneStore, FleetRuntimeGuard},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn fixture() -> Result<(tempfile::TempDir, SqliteControlPlane, FleetRuntimeGuard)> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(&directory.path().join("jobs.db")).await?;
    store.migrate().await?;
    seed(&store).await?;
    let control = SqliteControlPlane::new(store, directory.path().join("artifacts"));
    let guard =
        FleetRuntimeGuard::from(&control.fleet_get("forgejo").await?.ok_or("fleet missing")?);
    Ok((directory, control, guard))
}

async fn seed(store: &Store) -> Result {
    store.connection().execute_unprepared("INSERT INTO fleets
        (key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at)
        VALUES ('forgejo','inc',1,0,1,0,'Pending',0,1,1)").await?;
    let spec = serde_json::json!({"kind":"forgejo","forgejo":{
        "instance_url":"https://forgejo.test","scope":{"kind":"instance"},"auth_profile_ref":"token",
        "runner_name_prefix":"test-","labels":["linux:host"]},
        "capacity":{"min_runners":0,"max_runners":1},"template_profile_ref":"template"});
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO fleet_revisions (fleet_key,incarnation,revision,spec_json,auth_desired_profile_key,auth_desired_revision,inputs_digest,created_at)
        VALUES ('forgejo','inc',1,?,'token',1,'inputs',1)", [spec.to_string().into()])).await?;
    Ok(())
}
fn job() -> ForgejoJob {
    ForgejoJob {
        id: 7,
        repo_id: 9,
        attempt: 1,
        run_id: 8,
        task_id: 0,
        handle: "opaque".into(),
        status: "waiting".into(),
        runs_on: vec!["linux".into()],
        name: "build".into(),
    }
}
fn now() -> Result<i64> {
    Ok(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?)
}

#[tokio::test]
async fn forgejo_running_status_needs_no_verified_runner_and_disappearance_is_not_success() -> Result
{
    let (_directory, control, guard) = fixture().await?;
    let now = now()?;
    let mut task = job();
    assert!(
        control
            .forgejo_jobs_snapshot("forgejo", &guard, Some(&[task.clone()]), now)
            .await?
    );
    let page = control.store().list_jobs(JobsQuery::default()).await?;
    assert_eq!(page.items.len(), 1);
    let id = page.items[0].id.clone();
    assert_eq!(page.items[0].observed_status, ObservedStatus::Queued);
    assert_eq!(page.items[0].scale_set_id, None);
    assert_eq!(page.items[0].freshness, "fresh");
    task.status = "running".into();
    task.task_id = 42;
    control
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[task.clone()]), now + 1)
        .await?;
    control
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[task]), now + 2)
        .await?;
    let running = control.store().get_job(&id).await?.ok_or("job missing")?;
    assert_eq!(running.job.observed_status, ObservedStatus::Running);
    assert_eq!(
        running.job.association_status,
        AssociationStatus::Unverified
    );
    assert!(running.generations.is_empty());
    assert!(running.observations.is_empty());
    assert_eq!(
        running.forgejo_observations.len(),
        2,
        "unchanged polls are not new events"
    );
    control
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[]), now + 3)
        .await?;
    let gone = control.store().get_job(&id).await?.ok_or("job missing")?;
    assert_eq!(gone.job.observed_status, ObservedStatus::Unknown);
    assert_eq!(gone.job.reported_result, None);
    assert_eq!(gone.job.updated_at, now + 3);
    let state = gone.job.forgejo.ok_or("Forgejo state missing")?;
    assert_eq!(state.last_reported_status, "running");
    assert!(!state.in_snapshot);
    assert_eq!(gone.forgejo_observations.len(), 3);
    Ok(())
}

#[tokio::test]
async fn failed_and_stale_snapshots_preserve_facts_without_refreshing_them() -> Result {
    let (directory, control, guard) = fixture().await?;
    let at = now()?;
    control
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[job()]), at)
        .await?;
    control
        .forgejo_jobs_snapshot("forgejo", &guard, None, at + 2)
        .await?;
    assert!(
        !control
            .forgejo_jobs_snapshot("forgejo", &guard, Some(&[]), at + 1)
            .await?
    );
    let mut stale = guard.clone();
    stale.mutation_fence += 1;
    assert!(
        !control
            .forgejo_jobs_snapshot("forgejo", &stale, Some(&[]), at + 3)
            .await?
    );
    let reopened = Store::open(&directory.path().join("jobs.db")).await?;
    let job = reopened
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .remove(0);
    assert_eq!(job.observed_status, ObservedStatus::Queued);
    assert_eq!(job.freshness, "stale");
    assert_eq!(job.updated_at, at);
    assert_eq!(job.forgejo.ok_or("metadata")?.last_observed_at, at);
    Ok(())
}

#[tokio::test]
async fn repository_attempt_and_target_identities_do_not_merge_and_ids_remain_exact() -> Result {
    let (_directory, control, guard) = fixture().await?;
    let at = now()?;
    let tasks = [
        job(),
        ForgejoJob {
            repo_id: u64::MAX,
            ..job()
        },
        ForgejoJob {
            attempt: 2,
            ..job()
        },
    ];
    control
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&tasks), at)
        .await?;
    let first = control
        .store()
        .list_jobs(JobsQuery {
            limit: Some(2),
            ..Default::default()
        })
        .await?;
    assert_eq!(first.items.len(), 2);
    let second = control
        .store()
        .list_jobs(JobsQuery {
            limit: Some(2),
            cursor: first.next_cursor,
            ..Default::default()
        })
        .await?;
    assert_eq!(second.items.len(), 1);
    let all = control.store().list_jobs(JobsQuery::default()).await?;
    assert!(all.items.iter().any(|j| j
        .forgejo
        .as_ref()
        .is_some_and(|f| f.repository_id == "18446744073709551615")));
    let db = control.store().connection();
    for (path, value) in [
        (
            "$.forgejo.instance_url",
            serde_json::json!("https://other.test"),
        ),
        (
            "$.forgejo.scope",
            serde_json::json!({"kind":"organization","name":"org"}),
        ),
    ] {
        // Fixture-only mutation proves the namespace independently from Fleet admission.
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "UPDATE fleet_revisions SET spec_json=json_set(spec_json,?,json(?)) WHERE fleet_key='forgejo'",
            [path.into(), value.to_string().into()])).await?;
        control
            .forgejo_jobs_snapshot("forgejo", &guard, Some(&[job()]), at + 1)
            .await?;
    }
    assert_eq!(
        control
            .store()
            .list_jobs(JobsQuery::default())
            .await?
            .items
            .len(),
        5
    );
    Ok(())
}

#[tokio::test]
async fn invalid_snapshot_is_atomic_unknown_status_is_unknown_and_old_reads_expire() -> Result {
    let (_directory, control, guard) = fixture().await?;
    let at = now()? - 31_000;
    assert!(control
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[job(), job()]), at)
        .await
        .is_err());
    assert!(control
        .store()
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .is_empty());
    control
        .forgejo_jobs_snapshot(
            "forgejo",
            &guard,
            Some(&[ForgejoJob {
                status: "future-status".into(),
                ..job()
            }]),
            at,
        )
        .await?;
    let unknown = control
        .store()
        .list_jobs(JobsQuery::default())
        .await?
        .items
        .remove(0);
    assert_eq!(unknown.observed_status, ObservedStatus::Unknown);
    assert_eq!(unknown.freshness, "stale");
    assert_eq!(unknown.association_status, AssociationStatus::Unverified);
    control.store().prune_job_history(at + 1).await?;
    assert!(control.store().get_job(&unknown.id).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn github_and_forgejo_share_reads_not_assignment_or_scale_set_identities() -> Result {
    use shaula_core::ports::{JobMessage, PollMessage};
    let (store, context) = crate::tests::listener_messages::ready().await?;
    seed(&store).await?;
    store
        .listener_ingest(
            "fleet",
            &context,
            &PollMessage {
                message_id: 1,
                statistics: Default::default(),
                job_available: vec![JobMessage {
                    job_id: "9:7:1".into(),
                    runner_request_id: 1,
                    metadata: Default::default(),
                }],
                job_assigned: vec![],
                job_started: vec![],
                job_completed: vec![],
            },
            now()?,
        )
        .await?;
    let control = SqliteControlPlane::new(store, std::path::PathBuf::new());
    let guard = FleetRuntimeGuard::from(&control.fleet_get("forgejo").await?.ok_or("fleet")?);
    control
        .forgejo_jobs_snapshot("forgejo", &guard, Some(&[job()]), now()?)
        .await?;
    let page = control.store().list_jobs(JobsQuery::default()).await?;
    assert_eq!(page.items.len(), 2);
    assert_ne!(page.items[0].id, page.items[1].id);
    assert_eq!(
        page.items
            .iter()
            .filter(|j| j.scale_set_id.is_some())
            .count(),
        1
    );
    let filtered = control
        .store()
        .list_jobs(JobsQuery {
            fleet_key: Some("forgejo".into()),
            status: Some("queued".into()),
            ..Default::default()
        })
        .await?;
    assert_eq!(filtered.items.len(), 1);
    assert!(filtered.items[0].forgejo.is_some());
    Ok(())
}
