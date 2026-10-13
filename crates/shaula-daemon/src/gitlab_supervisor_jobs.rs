//! Read-only Jobs projection (spec 0043 §6.1). Projection failures are logged
//! and dropped: they never block demand, readiness or reclaim.
use super::GitlabPoolSupervisor;
use futures::StreamExt;
use shaula_core::{error::CoreResult, ports::gitlab::GitlabRunningJob, registry::GenerationRecord};

/// Concurrent exact reads per batch; the store caps a batch at twenty jobs.
const READ_CONCURRENCY: usize = 4;

impl GitlabPoolSupervisor {
    /// Records a job its own registration lists as running. The store links
    /// the Generation only when this job is its persisted Verified claim.
    pub(super) async fn project_running_job(
        &self,
        generation: &GenerationRecord,
        runner_id: u64,
        job: &GitlabRunningJob,
        now: i64,
    ) {
        if let Err(error) = self
            .lifecycle
            .gitlab_job_running(
                &self.fleet_key,
                &self.guard,
                &generation.id,
                runner_id,
                job,
                now,
            )
            .await
        {
            tracing::warn!(fleet = %self.fleet_key, generation = %generation.id,
                reason = error.code.as_str(), "GitLab job could not be projected");
        }
    }

    /// The wiring owns this read-only task under a different key from `tick`.
    /// It never holds the tick mutex, Fleet effect gate or SQLite writer during IO.
    pub async fn enrich_jobs(&self) -> CoreResult<()> {
        if self
            .current_head()
            .await?
            .is_none_or(|head| head.deletion_marker)
        {
            return Ok(());
        }
        let pending = self
            .lifecycle
            .gitlab_jobs_pending(&self.fleet_key, &self.guard, self.clock.now_unix_ms())
            .await?;
        futures::stream::iter(pending)
            .for_each_concurrent(READ_CONCURRENCY, |lookup| async move {
                let read = match self.gitlab.job(lookup.project_id, lookup.job_id).await {
                    Ok(read) => read,
                    Err(error) => {
                        tracing::debug!(fleet = %self.fleet_key, reason = crate::pool_driver::access_reason(&error).as_str(),
                            "GitLab job read unavailable; status remains as last observed");
                        return;
                    }
                };
                match self
                    .lifecycle
                    .gitlab_job_read(
                        &self.fleet_key,
                        &self.guard,
                        &lookup,
                        read.as_ref(),
                        self.clock.now_unix_ms(),
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => tracing::debug!(fleet = %self.fleet_key,
                        "GitLab job read was superseded or contradicts the claim; not applied"),
                    Err(error) => tracing::warn!(fleet = %self.fleet_key, reason = error.code.as_str(),
                        "GitLab job read could not be retained"),
                }
            })
            .await;
        Ok(())
    }
}
