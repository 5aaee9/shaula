//! Forgejo job IDs are scoped by instance/target/Fleet, repository and attempt.
use super::{decode, encode, execute, rows};
use crate::StoreResult;
use sea_orm::DatabaseTransaction;
use shaula_core::{
    fleet::FleetProviderKind,
    forgejo::{ForgejoScope, ForgejoTarget},
    jobs::{
        AssociationStatus, ForgejoJobObservation, ForgejoJobState, JobMetadata, JobSummary,
        ObservedStatus,
    },
    ports::forgejo::ForgejoJob,
};

pub(super) async fn upsert(
    tx: &DatabaseTransaction,
    fleet: &str,
    incarnation: &str,
    scope: &str,
    target: &ForgejoTarget,
    job: &ForgejoJob,
    now: i64,
) -> StoreResult<String> {
    let protocol_id = format!("{}:{}:{}", job.repo_id, job.id, job.attempt);
    let found = rows(
        tx,
        "SELECT summary_json FROM forgejo_workflow_jobs WHERE scope_key=? AND protocol_job_id=?",
        vec![scope.into(), protocol_id.clone().into()],
    )
    .await?;
    let previous: Option<JobSummary> = found
        .first()
        .map(|r| decode(&r.try_get::<String>("", "summary_json")?))
        .transpose()?;
    let prior = previous.as_ref().and_then(|j| j.forgejo.as_ref());
    let changed = prior.is_none_or(|old| {
        !old.in_snapshot
            || old.last_reported_status != job.status
            || old.task_id != job.task_id.to_string()
    });
    let status = match job.status.as_str() {
        "waiting" => ObservedStatus::Queued,
        "running" => ObservedStatus::Running,
        _ => ObservedStatus::Unknown,
    };
    let (owner, repository) = match &target.scope {
        ForgejoScope::Repository { owner, name } => (Some(owner.clone()), Some(name.clone())),
        _ => (None, None),
    };
    let repository_filter = owner
        .as_ref()
        .zip(repository.as_ref())
        .map(|(o, r)| format!("{o}/{r}"))
        .unwrap_or_else(|| format!("repository:{}", job.repo_id));
    let summary = JobSummary {
        id: previous
            .as_ref()
            .map(|j| j.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        fleet_key: fleet.into(),
        fleet_incarnation: incarnation.into(),
        backend: FleetProviderKind::Forgejo,
        scale_set_id: None,
        protocol_job_id: protocol_id.clone(),
        metadata: JobMetadata {
            job_display_name: Some(job.name.clone()),
            owner_name: owner,
            repository_name: repository,
            ..Default::default()
        },
        observed_status: status,
        reported_result: None,
        freshness: "unknown".into(),
        association_status: AssociationStatus::Unverified,
        github_run_url: None,
        actions_job_id: None,
        workflow_run_attempt: None,
        github_conclusion: None,
        created_at: previous.as_ref().map_or(now, |j| j.created_at),
        updated_at: now,
        forgejo: Some(ForgejoJobState {
            target: target.clone(),
            repository_id: job.repo_id.to_string(),
            job_id: job.id.to_string(),
            attempt: job.attempt.to_string(),
            run_id: job.run_id.to_string(),
            task_id: job.task_id.to_string(),
            runs_on: job.runs_on.clone(),
            last_reported_status: job.status.clone(),
            last_observed_at: now,
            in_snapshot: true,
        }),
    };
    execute(tx, "INSERT INTO forgejo_workflow_jobs(id,scope_key,fleet_key,fleet_incarnation,protocol_job_id,
        summary_json,status,repository,job_name,in_snapshot,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,1,?,?)
        ON CONFLICT(scope_key,protocol_job_id) DO UPDATE SET summary_json=excluded.summary_json,status=excluded.status,
        repository=excluded.repository,job_name=excluded.job_name,in_snapshot=1,updated_at=excluded.updated_at",
        vec![summary.id.clone().into(), scope.into(), fleet.into(), incarnation.into(), protocol_id.into(),
            encode(&summary)?.into(), status.as_str().into(), repository_filter.into(), job.name.clone().into(),
            summary.created_at.into(), now.into()]).await?;
    if changed {
        event(
            tx,
            &summary.id,
            Some(&job.status),
            &job.task_id.to_string(),
            now,
        )
        .await?;
    }
    Ok(summary.id)
}

pub(super) async fn event(
    tx: &DatabaseTransaction,
    id: &str,
    status: Option<&str>,
    task_id: &str,
    now: i64,
) -> StoreResult<()> {
    let observation = ForgejoJobObservation {
        id: uuid::Uuid::new_v4().to_string(),
        reported_status: status.map(str::to_owned),
        task_id: task_id.into(),
        observed_at: now,
    };
    execute(tx, "INSERT INTO forgejo_job_observations(id,job_record_id,data_json,observed_at) VALUES(?,?,?,?)",
        vec![observation.id.clone().into(), id.into(), encode(&observation)?.into(), now.into()]).await
}
