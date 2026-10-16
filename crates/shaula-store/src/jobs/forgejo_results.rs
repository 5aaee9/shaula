//! Optional history work is scoped, bounded and incapable of changing Runner state.
use super::{decode, encode, forgejo_projection, forgejo_scope};
use crate::entities::jobs::{forgejo_job_polls as polls, forgejo_workflow_jobs as jobs_table};
use crate::query_ext::json_extract;
use crate::{Store, StoreError, StoreResult};
use sea_orm::{
    sea_query::{Alias, Expr, Func, SimpleExpr},
    ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect,
};
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
        let reserved = polls::Entity::update_many()
            .col_expr(polls::Column::EnrichmentAttemptedAt, Expr::value(now))
            .filter(polls::Column::ScopeKey.eq(scope.key.as_str()))
            .filter(
                polls::Column::EnrichmentAttemptedAt
                    .eq(0)
                    .or(polls::Column::EnrichmentAttemptedAt.lte(now.saturating_sub(30_000))),
            )
            .exec(&tx)
            .await?;
        if reserved.rows_affected != 1 {
            return Ok(Vec::new());
        }
        let summary = jobs_table::Column::SummaryJson;
        let candidates = jobs_table::Entity::find()
            .select_only()
            .column(summary)
            .filter(jobs_table::Column::ScopeKey.eq(scope.key))
            .filter(jobs_table::Column::Status.ne("completed"))
            .filter(
                Expr::expr(Func::cast_as(
                    json_extract(summary, "$.forgejo.task_id"),
                    Alias::new("INTEGER"),
                ))
                .gt(0),
            )
            .order_by(
                SimpleExpr::from(Func::coalesce([
                    json_extract(summary, "$.forgejo.enrichment_attempted_at"),
                    Expr::val(0).into(),
                ])),
                Order::Asc,
            )
            .order_by_asc(jobs_table::Column::CreatedAt)
            .order_by_asc(jobs_table::Column::Id)
            .limit(100)
            .into_tuple::<String>()
            .all(&tx)
            .await?;
        let mut repositories = BTreeSet::new();
        let mut selected = Vec::new();
        for summary_json in candidates {
            let mut summary: JobSummary = decode(&summary_json)?;
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
            jobs_table::Entity::update_many()
                .col_expr(
                    jobs_table::Column::SummaryJson,
                    Expr::value(encode(&summary)?),
                )
                .filter(jobs_table::Column::Id.eq(summary.id))
                .exec(&tx)
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
        let found = jobs_table::Entity::find()
            .select_only()
            .column(jobs_table::Column::SummaryJson)
            .filter(jobs_table::Column::ScopeKey.eq(scope.key.as_str()))
            .filter(jobs_table::Column::Id.eq(lookup.record_id.as_str()))
            .into_tuple::<String>()
            .one(&tx)
            .await?;
        let Some(summary_json) = found else {
            return Ok(false);
        };
        let mut summary: JobSummary = decode(&summary_json)?;
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
        let summary_column = jobs_table::Column::SummaryJson;
        let identities = jobs_table::Entity::find()
            .select_only()
            .column(jobs_table::Column::Id)
            .filter(jobs_table::Column::ScopeKey.eq(scope.key))
            .filter(
                Expr::expr(json_extract(summary_column, "$.forgejo.repository_id"))
                    .eq(state.repository_id.as_str()),
            )
            .filter(
                Expr::expr(json_extract(summary_column, "$.forgejo.task_id"))
                    .eq(state.task_id.as_str()),
            )
            .limit(2)
            .into_tuple::<String>()
            .all(&tx)
            .await?;
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
        jobs_table::Entity::update_many()
            .col_expr(
                jobs_table::Column::SummaryJson,
                Expr::value(encode(&summary)?),
            )
            .col_expr(jobs_table::Column::Status, Expr::value("completed"))
            .col_expr(
                jobs_table::Column::Repository,
                Expr::value(format!("{}/{}", result.owner, result.repository)),
            )
            .col_expr(jobs_table::Column::UpdatedAt, Expr::value(now))
            .filter(jobs_table::Column::Id.eq(lookup.record_id.as_str()))
            .exec(&tx)
            .await?;
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
