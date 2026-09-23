//! Optional task-history worker, scheduled separately from the Runner tick.
use super::ForgejoPoolSupervisor;
use shaula_core::{error::CoreResult, jobs::ForgejoJobLookup};
use std::collections::BTreeMap;

impl ForgejoPoolSupervisor {
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
            .forgejo_jobs_pending_results(&self.fleet_key, &self.guard, self.clock.now_unix_ms())
            .await?;
        let mut repositories = BTreeMap::<u64, Vec<ForgejoJobLookup>>::new();
        for lookup in pending {
            repositories
                .entry(lookup.repository_id)
                .or_default()
                .push(lookup);
        }
        // The store caps a batch at four repositories / twenty exact task IDs.
        // Independent bounded reads cannot serialize or delay lifecycle work.
        let reads = repositories.into_iter().map(|(repository, lookups)| async move {
            let ids: Vec<_> = lookups.iter().map(|lookup| lookup.task_id).collect();
            let results = match self.forgejo.task_results(repository, &ids).await {
                Ok(results) => results,
                Err(error) => {
                    tracing::debug!(fleet = %self.fleet_key, reason = super::access_reason(&error).as_str(),
                        "Forgejo task history unavailable; results remain unknown");
                    return;
                }
            };
            for lookup in lookups {
                let Some(result) = results.iter().find(|result| {
                    result.repository_id == lookup.repository_id && result.task_id == lookup.task_id
                }) else {
                    continue;
                };
                if let Err(error) = self.lifecycle.forgejo_job_result(
                    &self.fleet_key, &self.guard, &lookup, result, self.clock.now_unix_ms()
                ).await {
                    tracing::warn!(fleet = %self.fleet_key, reason = error.code.as_str(),
                        "Forgejo task result could not be retained");
                }
            }
        });
        futures::future::join_all(reads).await;
        Ok(())
    }
}
