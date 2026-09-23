//! Read-time freshness expires even if the daemon stopped polling.
use super::{decode, rows};
use crate::StoreResult;
use sea_orm::{ConnectionTrait, QueryResult};
use shaula_core::jobs::{ForgejoJobObservation, JobDetail, JobSummary};

pub(super) fn freshness(job: &mut JobSummary, row: &QueryResult) {
    let Some(state) = &job.forgejo else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());
    let observed = row.try_get::<Option<i64>>("", "poll_time").ok().flatten();
    let failed = row.try_get::<Option<i64>>("", "poll_failed").ok().flatten();
    job.freshness = if state.result.is_some() {
        "confirmed"
    } else if !state.in_snapshot {
        "not_listed"
    } else if failed == Some(0)
        && observed
            .zip(now)
            .is_some_and(|(at, now)| now >= at && now <= at.saturating_add(30_000))
    {
        "fresh"
    } else {
        "stale"
    }
    .into();
}

pub(super) async fn detail<C: ConnectionTrait>(tx: &C, job: JobSummary) -> StoreResult<JobDetail> {
    let found = rows(tx, "SELECT data_json FROM forgejo_job_observations WHERE job_record_id=? ORDER BY observed_at DESC,id DESC LIMIT 1001", vec![job.id.clone().into()]).await?;
    let mut observations: Vec<ForgejoJobObservation> = found
        .iter()
        .map(|r| decode(&r.try_get::<String>("", "data_json")?))
        .collect::<StoreResult<_>>()?;
    let observations_truncated = observations.len() > 1000;
    observations.truncate(1000);
    Ok(JobDetail {
        job,
        observations: Vec::new(),
        forgejo_observations: observations,
        observations_truncated,
        generations: Vec::new(),
    })
}
