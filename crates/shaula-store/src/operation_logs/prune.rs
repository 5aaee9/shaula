//! Metadata expiry for empty invocations whose Generation and jobs are gone.
use sea_orm::sea_query::{
    Alias, CommonTableExpression, Expr, Query, UnionType, WithClause, WithQuery,
};

use crate::entities::{
    jobs::{
        workflow_generation_identity as identity, workflow_job_observations as observations,
        workflow_jobs as jobs,
    },
    lifecycle::runner_generations as generations,
    operation_logs::operation_log_invocations as log_invocations,
};
use crate::query_ext::json_extract;

/// Deletes sealed, empty invocations older than `cutoff` unless their
/// Generation is live or recent, or a recent job observation names it.
///
/// Both CTEs are `MATERIALIZED`: a per-invocation scan of every recent
/// observation took 30s+ on production volumes.
pub(super) fn metadata(cutoff: i64) -> WithQuery {
    let recent = Alias::new("recent");
    let protected = Alias::new("protected");
    let gid = Alias::new("gid");
    let recent_rows = Query::select()
        .expr_as(
            json_extract(
                (observations::Entity, observations::Column::DataJson),
                "$.generation_id",
            ),
            gid.clone(),
        )
        .column((observations::Entity, observations::Column::ScopeKey))
        .column((observations::Entity, observations::Column::RunnerId))
        .from(observations::Entity)
        .inner_join(
            jobs::Entity,
            Expr::col((jobs::Entity, jobs::Column::Id))
                .equals((observations::Entity, observations::Column::JobRecordId)),
        )
        .and_where(Expr::col((jobs::Entity, jobs::Column::UpdatedAt)).gte(cutoff))
        .to_owned();
    let by_runner = Query::select()
        .column((identity::Entity, identity::Column::GenerationId))
        .from(identity::Entity)
        .inner_join(
            recent.clone(),
            Expr::col((identity::Entity, identity::Column::ScopeKey))
                .equals((recent.clone(), observations::Column::ScopeKey))
                .and(
                    Expr::col((identity::Entity, identity::Column::GithubRunnerId))
                        .equals((recent.clone(), observations::Column::RunnerId)),
                ),
        )
        .to_owned();
    let protected_rows = Query::select()
        .column(gid.clone())
        .from(recent.clone())
        .and_where(Expr::col(gid.clone()).is_not_null())
        .union(UnionType::Distinct, by_runner)
        .to_owned();
    let live_generation = Query::select()
        .expr(Expr::val(1))
        .from(generations::Entity)
        .and_where(
            Expr::col((generations::Entity, generations::Column::Id)).equals((
                log_invocations::Entity,
                log_invocations::Column::GenerationId,
            )),
        )
        .and_where(
            Expr::col((generations::Entity, generations::Column::State))
                .ne("Destroyed")
                .or(Expr::col((generations::Entity, generations::Column::UpdatedAt)).gte(cutoff)),
        )
        .to_owned();
    let protected_generation = Query::select()
        .expr(Expr::val(1))
        .from(protected.clone())
        .and_where(Expr::col((protected.clone(), gid)).equals((
            log_invocations::Entity,
            log_invocations::Column::GenerationId,
        )))
        .to_owned();
    let delete = Query::delete()
        .from_table(log_invocations::Entity)
        .and_where(Expr::col(log_invocations::Column::SealedAt).lt(cutoff))
        .and_where(Expr::col(log_invocations::Column::RetainedBytes).eq(0))
        .and_where(Expr::exists(live_generation).not())
        .and_where(Expr::exists(protected_generation).not())
        .to_owned();
    WithClause::new()
        .cte(
            CommonTableExpression::new()
                .query(recent_rows)
                .table_name(recent)
                .materialized(true)
                .to_owned(),
        )
        .cte(
            CommonTableExpression::new()
                .query(protected_rows)
                .table_name(protected)
                .materialized(true)
                .to_owned(),
        )
        .to_owned()
        .query(delete)
}
