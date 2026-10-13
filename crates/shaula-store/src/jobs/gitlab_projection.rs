//! GitLab job IDs are scoped by Fleet incarnation/instance/scope/Auth Profile
//! and `project_id/job_id`. A recorded terminal result is never overwritten.
use super::{encode, execute, gitlab_scope::ScopeSnapshot};
use crate::{StoreError, StoreResult};
use sea_orm::DatabaseTransaction;
use shaula_core::{
    fleet::FleetProviderKind,
    jobs::{
        gitlab_observed_status, AssociationStatus, GitlabJobConclusion, GitlabJobObservation,
        GitlabJobResult, GitlabJobState, GitlabObservationSource, JobMetadata, JobSummary,
        ObservedStatus,
    },
};

/// One provider reading of a job, from the runner's jobs list or an exact read.
pub(super) struct Reading<'a> {
    pub project_id: u64,
    pub project_path: &'a str,
    pub job_id: u64,
    pub pipeline_id: u64,
    pub name: &'a str,
    pub status: &'a str,
    pub runner_id: Option<u64>,
    pub source: GitlabObservationSource,
}

impl Reading<'_> {
    pub fn protocol_id(&self) -> String {
        format!("{}:{}", self.project_id, self.job_id)
    }

    fn validate(&self) -> StoreResult<()> {
        let ids = [self.project_id, self.job_id, self.pipeline_id];
        let invalid = ids.iter().any(|id| *id == 0 || i64::try_from(*id).is_err())
            || self.runner_id.is_some_and(|id| id == 0)
            || self.name.is_empty()
            || self.name.len() > 1024
            || self.name.chars().any(char::is_control)
            || self.status.is_empty()
            || self.status.len() > 64
            || !self
                .status
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_');
        if invalid {
            return Err(StoreError::Corrupt("invalid GitLab job reading".into()));
        }
        Ok(())
    }
}

pub(super) struct Context<'a> {
    pub fleet: &'a str,
    pub incarnation: &'a str,
    pub scope: &'a ScopeSnapshot,
}

/// Upserts the record for `reading`; `previous` is the stored summary, if any.
pub(super) async fn apply(
    tx: &DatabaseTransaction,
    context: &Context<'_>,
    previous: Option<JobSummary>,
    reading: &Reading<'_>,
    now: i64,
) -> StoreResult<String> {
    reading.validate()?;
    let web_url = context
        .scope
        .target
        .job_url(reading.project_path, reading.job_id)
        .ok_or_else(|| StoreError::Corrupt("invalid GitLab project path".into()))?;
    let prior = previous.as_ref().and_then(|job| job.gitlab.as_ref());
    // A job's pipeline never changes; a different one is another identity.
    if prior.is_some_and(|old| old.pipeline_id != reading.pipeline_id.to_string()) {
        return Err(StoreError::Corrupt("GitLab job identity changed".into()));
    }
    let changed = prior.is_none_or(|old| !old.listed || old.last_reported_status != reading.status);
    let result = prior.and_then(|old| old.result.clone()).or_else(|| {
        GitlabJobConclusion::parse(reading.status).map(|status| GitlabJobResult {
            status,
            observed_at: now,
        })
    });
    let status = if result.is_some() {
        ObservedStatus::Completed
    } else {
        gitlab_observed_status(reading.status)
    };
    let summary = JobSummary {
        id: previous
            .as_ref()
            .map(|job| job.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        fleet_key: context.fleet.into(),
        fleet_incarnation: context.incarnation.into(),
        backend: FleetProviderKind::Gitlab,
        forgejo: None,
        scale_set_id: None,
        protocol_job_id: reading.protocol_id(),
        metadata: JobMetadata {
            job_display_name: Some(reading.name.into()),
            ..Default::default()
        },
        observed_status: status,
        reported_result: result.as_ref().map(|r| r.status.as_str().into()),
        freshness: "unknown".into(),
        association_status: AssociationStatus::Verified,
        github_run_url: None,
        actions_job_id: None,
        workflow_run_attempt: None,
        github_conclusion: None,
        created_at: previous.as_ref().map_or(now, |job| job.created_at),
        updated_at: previous.as_ref().map_or(now, |job| now.max(job.updated_at)),
        gitlab: Some(GitlabJobState {
            target: context.scope.target.clone(),
            project_id: reading.project_id.to_string(),
            project_path: reading.project_path.into(),
            job_id: reading.job_id.to_string(),
            pipeline_id: reading.pipeline_id.to_string(),
            job_name: reading.name.into(),
            web_url,
            last_reported_status: reading.status.into(),
            last_observed_at: now,
            listed: true,
            result,
            enrichment_attempted_at: prior.and_then(|old| old.enrichment_attempted_at),
        }),
    };
    execute(tx, "INSERT INTO gitlab_workflow_jobs(id,scope_key,fleet_key,fleet_incarnation,protocol_job_id,
        summary_json,status,repository,job_name,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)
        ON CONFLICT(scope_key,protocol_job_id) DO UPDATE SET summary_json=excluded.summary_json,status=excluded.status,
        repository=excluded.repository,job_name=excluded.job_name,updated_at=excluded.updated_at",
        vec![summary.id.clone().into(), context.scope.key.clone().into(), context.fleet.into(),
            context.incarnation.into(), reading.protocol_id().into(), encode(&summary)?.into(),
            status.as_str().into(), reading.project_path.into(), reading.name.into(),
            summary.created_at.into(), summary.updated_at.into()]).await?;
    if changed {
        event(
            tx,
            &summary.id,
            Some(reading.status),
            reading.source,
            reading.runner_id,
            now,
        )
        .await?;
    }
    Ok(summary.id)
}

/// The exact read no longer finds the job; absence is never a result.
pub(super) async fn unlisted(
    tx: &DatabaseTransaction,
    mut summary: JobSummary,
    now: i64,
) -> StoreResult<()> {
    let state = summary
        .gitlab
        .as_mut()
        .ok_or_else(|| StoreError::Corrupt("GitLab job metadata missing".into()))?;
    if state.result.is_some() || !state.listed {
        return Ok(());
    }
    state.listed = false;
    summary.observed_status = ObservedStatus::Unknown;
    summary.updated_at = summary.updated_at.max(now);
    execute(
        tx,
        "UPDATE gitlab_workflow_jobs SET summary_json=?,status=?,updated_at=? WHERE id=?",
        vec![
            encode(&summary)?.into(),
            summary.observed_status.as_str().into(),
            summary.updated_at.into(),
            summary.id.clone().into(),
        ],
    )
    .await?;
    event(
        tx,
        &summary.id,
        None,
        GitlabObservationSource::JobRead,
        None,
        now,
    )
    .await
}

async fn event(
    tx: &DatabaseTransaction,
    id: &str,
    status: Option<&str>,
    source: GitlabObservationSource,
    runner_id: Option<u64>,
    now: i64,
) -> StoreResult<()> {
    let observation = GitlabJobObservation {
        id: uuid::Uuid::new_v4().to_string(),
        reported_status: status.map(str::to_owned),
        source,
        runner_id: runner_id.map(|id| id.to_string()),
        observed_at: now,
    };
    execute(tx, "INSERT INTO gitlab_job_observations(id,job_record_id,data_json,observed_at) VALUES(?,?,?,?)",
        vec![observation.id.clone().into(), id.into(), encode(&observation)?.into(), now.into()]).await
}
