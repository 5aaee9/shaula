//! Forgejo job IDs are scoped by instance/target/Fleet, repository and attempt.
use super::{decode, encode};
use crate::entities::jobs::{
    forgejo_job_observations as observations, forgejo_workflow_jobs as jobs_table,
};
use crate::StoreResult;
use sea_orm::{
    sea_query::OnConflict, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait,
    QueryFilter, QuerySelect,
};
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
    let previous: Option<JobSummary> = jobs_table::Entity::find()
        .select_only()
        .column(jobs_table::Column::SummaryJson)
        .filter(jobs_table::Column::ScopeKey.eq(scope))
        .filter(jobs_table::Column::ProtocolJobId.eq(protocol_id.as_str()))
        .into_tuple::<String>()
        .one(tx)
        .await?
        .map(|json| decode(&json))
        .transpose()?;
    let prior = previous.as_ref().and_then(|j| j.forgejo.as_ref());
    let changed = prior.is_none_or(|old| {
        !old.in_snapshot
            || old.last_reported_status != job.status
            || old.task_id != job.task_id.to_string()
    });
    let retained = prior.filter(|old| old.task_id == job.task_id.to_string());
    let result = retained.and_then(|old| old.result.clone());
    let status = if result.is_some() {
        ObservedStatus::Completed
    } else {
        match job.status.as_str() {
            "waiting" => ObservedStatus::Queued,
            "running" => ObservedStatus::Running,
            _ => ObservedStatus::Unknown,
        }
    };
    let (owner, repository) = match &target.scope {
        ForgejoScope::Repository { owner, name } => (Some(owner.clone()), Some(name.clone())),
        _ => previous
            .as_ref()
            .map(|j| {
                (
                    j.metadata.owner_name.clone(),
                    j.metadata.repository_name.clone(),
                )
            })
            .unwrap_or_default(),
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
        gitlab: None,
        scale_set_id: None,
        protocol_job_id: protocol_id.clone(),
        metadata: JobMetadata {
            job_display_name: Some(job.name.clone()),
            owner_name: owner,
            repository_name: repository,
            ..Default::default()
        },
        observed_status: status,
        reported_result: result.as_ref().map(|r| r.conclusion.as_str().into()),
        freshness: "unknown".into(),
        association_status: AssociationStatus::Unverified,
        github_run_url: None,
        actions_job_id: None,
        workflow_run_attempt: None,
        github_conclusion: None,
        created_at: previous.as_ref().map_or(now, |j| j.created_at),
        updated_at: previous.as_ref().map_or(now, |old| now.max(old.updated_at)),
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
            result,
            enrichment_attempted_at: retained.and_then(|old| old.enrichment_attempted_at),
        }),
    };
    jobs_table::Entity::insert(jobs_table::ActiveModel {
        id: Set(summary.id.clone()),
        scope_key: Set(scope.into()),
        fleet_key: Set(fleet.into()),
        fleet_incarnation: Set(incarnation.into()),
        protocol_job_id: Set(protocol_id),
        summary_json: Set(encode(&summary)?),
        status: Set(status.as_str().into()),
        repository: Set(Some(repository_filter)),
        job_name: Set(Some(job.name.clone())),
        in_snapshot: Set(true),
        created_at: Set(summary.created_at),
        updated_at: Set(summary.updated_at),
    })
    .on_conflict(
        OnConflict::columns([
            jobs_table::Column::ScopeKey,
            jobs_table::Column::ProtocolJobId,
        ])
        .update_columns([
            jobs_table::Column::SummaryJson,
            jobs_table::Column::Status,
            jobs_table::Column::Repository,
            jobs_table::Column::JobName,
        ])
        .value(jobs_table::Column::InSnapshot, true)
        .update_column(jobs_table::Column::UpdatedAt)
        .to_owned(),
    )
    .exec_without_returning(tx)
    .await?;
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
    append_event(
        tx,
        id,
        status,
        task_id,
        now,
        shaula_core::jobs::ForgejoObservationSource::RunnerSnapshot,
    )
    .await
}

pub(super) async fn result_event(
    tx: &DatabaseTransaction,
    id: &str,
    result: &shaula_core::jobs::ForgejoTaskResult,
    now: i64,
) -> StoreResult<()> {
    append_event(
        tx,
        id,
        Some(result.conclusion.as_str()),
        &result.task_id.to_string(),
        now,
        shaula_core::jobs::ForgejoObservationSource::TaskHistory,
    )
    .await
}

async fn append_event(
    tx: &DatabaseTransaction,
    id: &str,
    status: Option<&str>,
    task_id: &str,
    now: i64,
    source: shaula_core::jobs::ForgejoObservationSource,
) -> StoreResult<()> {
    let observation = ForgejoJobObservation {
        id: uuid::Uuid::new_v4().to_string(),
        reported_status: status.map(str::to_owned),
        task_id: task_id.into(),
        observed_at: now,
        source,
    };
    observations::Entity::insert(observations::ActiveModel {
        id: Set(observation.id.clone()),
        job_record_id: Set(id.into()),
        data_json: Set(encode(&observation)?),
        observed_at: Set(now),
    })
    .exec_without_returning(tx)
    .await?;
    Ok(())
}
