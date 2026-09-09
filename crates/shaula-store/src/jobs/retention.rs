//! Bounded metadata expiry never deletes lifecycle ledgers or retained log records.
use super::{execute, rows};
use crate::{Store, StoreResult};

impl Store {
    /// `cutoff_ms` is now minus the configured metadata retention window.
    /// Unknown jobs can age out; their expiry does not assert a GitHub result.
    pub async fn prune_job_history(&self, cutoff_ms: i64) -> StoreResult<()> {
        let tx = self.begin().await?;
        let candidates = rows(&tx, "SELECT j.id FROM workflow_jobs j WHERE j.updated_at<?
            AND NOT EXISTS(SELECT 1 FROM workflow_job_observations o
                JOIN workflow_generation_identity i ON i.scope_key=o.scope_key
                    AND (i.generation_id=json_extract(o.data_json,'$.generation_id')
                        OR (o.runner_id>0 AND i.github_runner_id=o.runner_id))
                JOIN runner_generations g ON g.id=i.generation_id WHERE o.job_record_id=j.id
                AND (g.state<>'Destroyed' OR g.updated_at>=?
                    OR EXISTS(SELECT 1 FROM operation_log_invocations l WHERE l.generation_id=g.id)))
            ORDER BY j.updated_at,j.id LIMIT 1000", vec![cutoff_ms.into(), cutoff_ms.into()]).await?;
        for row in candidates {
            let id: String = row.try_get("", "id")?;
            execute(
                &tx,
                "DELETE FROM workflow_job_observations WHERE job_record_id=?",
                vec![id.clone().into()],
            )
            .await?;
            execute(&tx, "DELETE FROM workflow_jobs WHERE id=?", vec![id.into()]).await?;
        }
        execute(&tx, "DELETE FROM workflow_job_observations WHERE id IN (
            SELECT o.id FROM workflow_job_observations o WHERE o.job_record_id IS NULL AND o.observed_at<?
            AND NOT EXISTS(SELECT 1 FROM workflow_generation_identity i
                JOIN runner_generations g ON g.id=i.generation_id WHERE i.scope_key=o.scope_key
                AND (i.generation_id=json_extract(o.data_json,'$.generation_id')
                    OR (o.runner_id>0 AND i.github_runner_id=o.runner_id))
                AND (g.state<>'Destroyed' OR g.updated_at>=?
                    OR EXISTS(SELECT 1 FROM operation_log_invocations l WHERE l.generation_id=g.id)))
            ORDER BY o.observed_at,o.id LIMIT 1000)", vec![cutoff_ms.into(), cutoff_ms.into()]).await?;
        tx.commit().await?;
        Ok(())
    }
}
