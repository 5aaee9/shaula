//! Optional exact job reads: scoped, bounded and incapable of changing Runner
//! state or a recorded Verified association.
use super::{decode, encode, gitlab::find, gitlab_projection, gitlab_scope};
use crate::entities::jobs::{
    gitlab_job_generations as links, gitlab_job_polls as polls, gitlab_workflow_jobs as jobs_table,
};
use crate::query_ext::{col, json_extract};
use crate::{Store, StoreError, StoreResult};
use sea_orm::{
    sea_query::{Expr, Func, OnConflict, SimpleExpr},
    ActiveValue::Set,
    ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect,
};
use shaula_core::{
    jobs::{GitlabJobLookup, GitlabObservationSource, JobSummary},
    ports::gitlab::GitlabJobDetail,
    registry::FleetRuntimeGuard,
};

/// At most one batch per scope per interval, and at most this many jobs.
const READ_INTERVAL_MS: i64 = 30_000;
const READ_BATCH: u64 = 20;

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
        let reserved = polls::Entity::insert(polls::ActiveModel {
            scope_key: Set(scope.key.clone()),
            enrichment_attempted_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(polls::Column::ScopeKey)
                .update_column(polls::Column::EnrichmentAttemptedAt)
                .action_and_where(
                    col(polls::Column::EnrichmentAttemptedAt)
                        .lte(now.saturating_sub(READ_INTERVAL_MS)),
                )
                .to_owned(),
        )
        .exec_without_returning(&tx)
        .await?;
        if reserved != 1 {
            return Ok(Vec::new());
        }
        let candidates = jobs_table::Entity::find()
            .select_only()
            .column(jobs_table::Column::SummaryJson)
            .filter(jobs_table::Column::ScopeKey.eq(scope.key))
            .filter(jobs_table::Column::Status.ne("completed"))
            .order_by(
                SimpleExpr::from(Func::coalesce([
                    json_extract(
                        jobs_table::Column::SummaryJson,
                        "$.gitlab.enrichment_attempted_at",
                    ),
                    Expr::val(0).into(),
                ])),
                Order::Asc,
            )
            .order_by_asc(jobs_table::Column::CreatedAt)
            .order_by_asc(jobs_table::Column::Id)
            .limit(READ_BATCH)
            .into_tuple::<String>()
            .all(&tx)
            .await?;
        let mut selected = Vec::new();
        for summary_json in candidates {
            let mut summary: JobSummary = decode(&summary_json)?;
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
            jobs_table::Entity::update_many()
                .col_expr(
                    jobs_table::Column::SummaryJson,
                    Expr::value(encode(&summary)?),
                )
                .filter(jobs_table::Column::Id.eq(summary.id))
                .exec(&tx)
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
            let linked = links::Entity::find()
                .select_only()
                .column(links::Column::RunnerId)
                .filter(links::Column::JobRecordId.eq(summary.id.as_str()))
                .into_tuple::<i64>()
                .all(&tx)
                .await?;
            let runner = super::gitlab::to_i64(runner)?;
            if !linked.iter().all(|id| *id == runner) {
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
