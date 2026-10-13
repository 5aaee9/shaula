//! Read-time freshness expires even if the daemon stopped reading; the
//! Generation list holds only append-only Verified links.
use super::{decode, read::generation_summary, read::GENERATIONS, rows};
use crate::StoreResult;
use sea_orm::ConnectionTrait;
use shaula_core::jobs::{GitlabJobObservation, JobDetail, JobSummary};

pub(super) fn freshness(job: &mut JobSummary) {
    let Some(state) = &job.gitlab else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());
    job.freshness = if state.result.is_some() {
        "confirmed"
    } else if !state.listed {
        "not_listed"
    } else if now.is_some_and(|now| {
        now >= state.last_observed_at && now <= state.last_observed_at.saturating_add(30_000)
    }) {
        "fresh"
    } else {
        "stale"
    }
    .into();
}

pub(super) async fn detail<C: ConnectionTrait>(tx: &C, job: JobSummary) -> StoreResult<JobDetail> {
    let found = rows(tx, "SELECT data_json FROM gitlab_job_observations WHERE job_record_id=? ORDER BY observed_at DESC,id DESC LIMIT 1001", vec![job.id.clone().into()]).await?;
    let mut observations: Vec<GitlabJobObservation> = found
        .iter()
        .map(|r| decode(&r.try_get::<String>("", "data_json")?))
        .collect::<StoreResult<_>>()?;
    let observations_truncated = observations.len() > 1000;
    observations.truncate(1000);
    let sql = format!(
        "SELECT * FROM ({GENERATIONS}) WHERE id IN (SELECT generation_id FROM gitlab_job_generations WHERE job_record_id=?) ORDER BY created_at,id LIMIT 200"
    );
    let generations = rows(tx, &sql, vec![job.id.clone().into()])
        .await?
        .iter()
        .map(generation_summary)
        .collect::<StoreResult<_>>()?;
    Ok(JobDetail {
        job,
        observations: Vec::new(),
        forgejo_observations: Vec::new(),
        gitlab_observations: observations,
        observations_truncated,
        generations,
    })
}

/// Jobs a GitLab Generation's registration ran, by its Verified links.
pub(super) async fn generation_jobs<C: ConnectionTrait>(
    tx: &C,
    generation: &str,
) -> StoreResult<Vec<JobSummary>> {
    rows(
        tx,
        "SELECT j.summary_json FROM gitlab_workflow_jobs j WHERE j.id IN (
        SELECT job_record_id FROM gitlab_job_generations WHERE generation_id=?)
        ORDER BY j.created_at DESC,j.id DESC LIMIT 200",
        vec![generation.into()],
    )
    .await?
    .iter()
    .map(|row| {
        let mut job: JobSummary = decode(&row.try_get::<String>("", "summary_json")?)?;
        freshness(&mut job);
        Ok(job)
    })
    .collect()
}
