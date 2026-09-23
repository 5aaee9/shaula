//! Optional history work is scoped, bounded and incapable of changing Runner state.
use super::{decode, encode, execute, forgejo_projection, forgejo_scope, rows};
use crate::{Store, StoreError, StoreResult};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::{
    jobs::{ForgejoJobLookup, ForgejoJobResult, ForgejoTaskResult, JobSummary, ObservedStatus},
    registry::FleetRuntimeGuard,
};
use std::collections::BTreeSet;

impl Store {
    pub(super) async fn pending_forgejo_results(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        now: i64,
    ) -> StoreResult<Vec<ForgejoJobLookup>> {
        let tx = self.begin().await?;
        let Some(scope) = forgejo_scope::current(&tx, fleet, guard).await? else {
            return Ok(Vec::new());
        };
        // This is a read-work budget, not an execution claim. It survives failure,
        // cancellation and restart; no network operation occurs inside the writer.
        let reserved = tx
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE forgejo_job_polls SET enrichment_attempted_at=? WHERE scope_key=?
             AND (enrichment_attempted_at=0 OR enrichment_attempted_at<=?)",
                [
                    now.into(),
                    scope.key.clone().into(),
                    now.saturating_sub(30_000).into(),
                ],
            ))
            .await?;
        if reserved.rows_affected() != 1 {
            return Ok(Vec::new());
        }
        let candidates = rows(&tx,
            "SELECT id,summary_json FROM forgejo_workflow_jobs WHERE scope_key=? AND status!='completed'
             AND CAST(json_extract(summary_json,'$.forgejo.task_id') AS INTEGER)>0
             ORDER BY COALESCE(json_extract(summary_json,'$.forgejo.enrichment_attempted_at'),0),created_at,id LIMIT 100",
            vec![scope.key.into()],
        ).await?;
        let mut repositories = BTreeSet::new();
        let mut selected = Vec::new();
        for row in candidates {
            let mut summary: JobSummary = decode(&row.try_get::<String>("", "summary_json")?)?;
            let state = summary
                .forgejo
                .as_mut()
                .ok_or_else(|| StoreError::Corrupt("Forgejo job metadata missing".into()))?;
            let repository_id = decimal_id(&state.repository_id)?;
            if !repositories.contains(&repository_id) && repositories.len() >= 4 {
                continue;
            }
            repositories.insert(repository_id);
            selected.push(ForgejoJobLookup {
                record_id: summary.id.clone(),
                repository_id,
                task_id: decimal_id(&state.task_id)?,
            });
            state.enrichment_attempted_at = Some(now);
            execute(
                &tx,
                "UPDATE forgejo_workflow_jobs SET summary_json=? WHERE id=?",
                vec![encode(&summary)?.into(), summary.id.into()],
            )
            .await?;
            if selected.len() == 20 {
                break;
            }
        }
        tx.commit().await?;
        Ok(selected)
    }

    pub(super) async fn apply_forgejo_result(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        lookup: &ForgejoJobLookup,
        result: &ForgejoTaskResult,
        now: i64,
    ) -> StoreResult<bool> {
        if lookup.task_id == 0
            || lookup.repository_id == 0
            || lookup.task_id != result.task_id
            || lookup.repository_id != result.repository_id
        {
            return Ok(false);
        }
        let tx = self.begin().await?;
        let Some(scope) = forgejo_scope::current(&tx, fleet, guard).await? else {
            return Ok(false);
        };
        result
            .validate_for_instance(&scope.target.instance_url)
            .map_err(|message| StoreError::Corrupt(message.into()))?;
        let found = rows(
            &tx,
            "SELECT summary_json FROM forgejo_workflow_jobs WHERE scope_key=? AND id=?",
            vec![scope.key.clone().into(), lookup.record_id.clone().into()],
        )
        .await?;
        let Some(row) = found.first() else {
            return Ok(false);
        };
        let mut summary: JobSummary = decode(&row.try_get::<String>("", "summary_json")?)?;
        let state = summary
            .forgejo
            .as_mut()
            .ok_or_else(|| StoreError::Corrupt("Forgejo job metadata missing".into()))?;
        if state.task_id != result.task_id.to_string()
            || state.repository_id != result.repository_id.to_string()
            || now < summary.updated_at
        {
            return Ok(false);
        }
        // Two different job/attempt records claiming the same task are ambiguous.
        // Matching a task ID alone must not spread a conclusion across those rows.
        let identities = rows(&tx,
            "SELECT id FROM forgejo_workflow_jobs WHERE scope_key=? AND json_extract(summary_json,'$.forgejo.repository_id')=?
             AND json_extract(summary_json,'$.forgejo.task_id')=? LIMIT 2",
            vec![scope.key.into(),state.repository_id.clone().into(),state.task_id.clone().into()],
        ).await?;
        if identities.len() != 1 {
            return Ok(false);
        }
        if let Some(previous) = &state.result {
            return Ok(
                previous.task_id == state.task_id && previous.conclusion == result.conclusion
            );
        }
        state.result = Some(ForgejoJobResult {
            task_id: state.task_id.clone(),
            conclusion: result.conclusion,
            observed_at: now,
            run_number: result.run_number.to_string(),
            run_url: result.run_url.clone(),
            workflow: result.workflow.clone(),
        });
        summary.observed_status = ObservedStatus::Completed;
        summary.reported_result = Some(result.conclusion.as_str().into());
        summary.metadata.owner_name = Some(result.owner.clone());
        summary.metadata.repository_name = Some(result.repository.clone());
        summary.updated_at = now;
        execute(&tx,"UPDATE forgejo_workflow_jobs SET summary_json=?,status='completed',repository=?,updated_at=? WHERE id=?",
            vec![encode(&summary)?.into(),format!("{}/{}",result.owner,result.repository).into(),now.into(),lookup.record_id.clone().into()],
        ).await?;
        forgejo_projection::result_event(&tx, &lookup.record_id, result, now).await?;
        tx.commit().await?;
        Ok(true)
    }
}

fn decimal_id(value: &str) -> StoreResult<u64> {
    value
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == value)
        .ok_or_else(|| StoreError::Corrupt("invalid Forgejo task identity".into()))
}
