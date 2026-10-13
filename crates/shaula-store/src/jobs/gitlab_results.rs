//! Optional exact job reads: scoped, bounded and incapable of changing Runner
//! state or a recorded Verified association.
use super::{decode, encode, execute, gitlab::find, gitlab_projection, gitlab_scope, rows};
use crate::{Store, StoreError, StoreResult};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::{
    jobs::{GitlabJobLookup, GitlabObservationSource, JobSummary},
    ports::gitlab::GitlabJobDetail,
    registry::FleetRuntimeGuard,
};

/// At most one batch per scope per interval, and at most this many jobs.
const READ_INTERVAL_MS: i64 = 30_000;
const READ_BATCH: usize = 20;

impl Store {
    pub(super) async fn pending_gitlab_reads(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        now: i64,
    ) -> StoreResult<Vec<GitlabJobLookup>> {
        let tx = self.begin().await?;
        let Some(scope) = gitlab_scope::current(&tx, fleet, guard).await? else {
            return Ok(Vec::new());
        };
        // A read-work budget, not an execution claim. It survives failure,
        // cancellation and restart; no network operation occurs inside the writer.
        let reserved = tx
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "INSERT INTO gitlab_job_polls(scope_key,enrichment_attempted_at) VALUES(?,?)
                 ON CONFLICT(scope_key) DO UPDATE SET enrichment_attempted_at=excluded.enrichment_attempted_at
                 WHERE gitlab_job_polls.enrichment_attempted_at<=?",
                [
                    scope.key.clone().into(),
                    now.into(),
                    now.saturating_sub(READ_INTERVAL_MS).into(),
                ],
            ))
            .await?;
        if reserved.rows_affected() != 1 {
            return Ok(Vec::new());
        }
        let candidates = rows(&tx,
            "SELECT summary_json FROM gitlab_workflow_jobs WHERE scope_key=? AND status!='completed'
             ORDER BY COALESCE(json_extract(summary_json,'$.gitlab.enrichment_attempted_at'),0),created_at,id LIMIT ?",
            vec![scope.key.into(), (READ_BATCH as i64).into()],
        ).await?;
        let mut selected = Vec::new();
        for row in candidates {
            let mut summary: JobSummary = decode(&row.try_get::<String>("", "summary_json")?)?;
            let state = summary
                .gitlab
                .as_mut()
                .ok_or_else(|| StoreError::Corrupt("GitLab job metadata missing".into()))?;
            selected.push(GitlabJobLookup {
                record_id: summary.id.clone(),
                project_id: decimal_id(&state.project_id)?,
                job_id: decimal_id(&state.job_id)?,
            });
            state.enrichment_attempted_at = Some(now);
            execute(
                &tx,
                "UPDATE gitlab_workflow_jobs SET summary_json=? WHERE id=?",
                vec![encode(&summary)?.into(), summary.id.into()],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(selected)
    }

    pub(super) async fn apply_gitlab_read(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        lookup: &GitlabJobLookup,
        read: Option<&GitlabJobDetail>,
        now: i64,
    ) -> StoreResult<bool> {
        if read.is_some_and(|r| r.id != lookup.job_id || r.project_id != lookup.project_id) {
            return Ok(false);
        }
        let tx = self.begin().await?;
        let Some(scope) = gitlab_scope::current(&tx, fleet, guard).await? else {
            return Ok(false);
        };
        let protocol_id = format!("{}:{}", lookup.project_id, lookup.job_id);
        let Some(summary) = find(&tx, &scope.key, &protocol_id).await? else {
            return Ok(false);
        };
        let state = summary
            .gitlab
            .as_ref()
            .ok_or_else(|| StoreError::Corrupt("GitLab job metadata missing".into()))?;
        if summary.id != lookup.record_id || now < summary.updated_at {
            return Ok(false);
        }
        let Some(read) = read else {
            gitlab_projection::unlisted(&tx, summary, now).await?;
            tx.commit().await?;
            return Ok(true);
        };
        if let Some(previous) = &state.result {
            return Ok(previous.status.as_str() == read.status);
        }
        // `runner: null` (deleted registration) is no evidence either way; a
        // different registration contradicts the claim and is not applied.
        if let Some(runner) = read.runner_id {
            let linked = rows(
                &tx,
                "SELECT runner_id FROM gitlab_job_generations WHERE job_record_id=?",
                vec![summary.id.clone().into()],
            )
            .await?;
            let runner = super::gitlab::to_i64(runner)?;
            if !linked.iter().all(|row| {
                row.try_get::<i64>("", "runner_id")
                    .is_ok_and(|id| id == runner)
            }) {
                return Ok(false);
            }
        }
        let reading = gitlab_projection::Reading {
            project_id: read.project_id,
            project_path: &read.project_path,
            job_id: read.id,
            pipeline_id: read.pipeline_id,
            name: &read.name,
            status: &read.status,
            runner_id: read.runner_id,
            source: GitlabObservationSource::JobRead,
        };
        let context = gitlab_projection::Context {
            fleet,
            incarnation: &guard.incarnation,
            scope: &scope,
        };
        gitlab_projection::apply(&tx, &context, Some(summary), &reading, now).await?;
        tx.commit().await?;
        Ok(true)
    }
}

fn decimal_id(value: &str) -> StoreResult<u64> {
    value
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == value)
        .ok_or_else(|| StoreError::Corrupt("invalid GitLab job identity".into()))
}
