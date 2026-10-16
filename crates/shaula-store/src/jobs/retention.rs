//! Bounded metadata expiry never deletes lifecycle ledgers or retained log records.
use sea_orm::{
    sea_query::{Expr, SimpleExpr},
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
};

use super::github_generation_evidence;
use crate::entities::{
    jobs::{
        forgejo_job_observations as forgejo_observations, forgejo_job_polls as forgejo_polls,
        forgejo_workflow_jobs as forgejo_jobs, gitlab_job_generations as gitlab_links,
        gitlab_job_observations as gitlab_observations, gitlab_job_polls as gitlab_polls,
        gitlab_workflow_jobs as gitlab_jobs, workflow_generation_identity as identity,
        workflow_job_observations as github_observations, workflow_jobs as github_jobs,
    },
    lifecycle::runner_generations as generations,
    operation_logs::operation_log_invocations as log_invocations,
};
use crate::query_ext::{col, probe, same};
use crate::{Store, StoreResult};

/// Rows kept per table per pass; each pass is one bounded writer transaction.
const BATCH: u64 = 1000;

/// The joined Generation is live, recently Destroyed, or still retains Operation Logs.
fn generation_current(cutoff_ms: i64) -> SimpleExpr {
    col(generations::Column::State)
        .ne("Destroyed")
        .or(col(generations::Column::UpdatedAt).gte(cutoff_ms))
        .or(Expr::exists(
            probe(log_invocations::Entity)
                .and_where(same(
                    log_invocations::Column::GenerationId,
                    generations::Column::Id,
                ))
                .to_owned(),
        ))
}

impl Store {
    /// `cutoff_ms` is now minus the configured metadata retention window.
    /// Unknown jobs can age out; their expiry does not assert a GitHub result.
    pub async fn prune_job_history(&self, cutoff_ms: i64) -> StoreResult<()> {
        let tx = self.begin().await?;
        let protected = probe(github_observations::Entity)
            .inner_join(
                identity::Entity,
                same(
                    identity::Column::ScopeKey,
                    github_observations::Column::ScopeKey,
                )
                .and(github_generation_evidence()),
            )
            .inner_join(
                generations::Entity,
                same(generations::Column::Id, identity::Column::GenerationId),
            )
            .and_where(same(
                github_observations::Column::JobRecordId,
                github_jobs::Column::Id,
            ))
            .and_where(generation_current(cutoff_ms))
            .to_owned();
        let candidates = github_jobs::Entity::find()
            .select_only()
            .column(github_jobs::Column::Id)
            .filter(github_jobs::Column::UpdatedAt.lt(cutoff_ms))
            .filter(Expr::exists(protected).not())
            .order_by_asc(github_jobs::Column::UpdatedAt)
            .order_by_asc(github_jobs::Column::Id)
            .limit(BATCH)
            .into_tuple::<String>()
            .all(&tx)
            .await?;
        for id in candidates {
            github_observations::Entity::delete_many()
                .filter(github_observations::Column::JobRecordId.eq(id.as_str()))
                .exec(&tx)
                .await?;
            github_jobs::Entity::delete_by_id(id).exec(&tx).await?;
        }
        let referenced = probe(identity::Entity)
            .inner_join(
                generations::Entity,
                same(generations::Column::Id, identity::Column::GenerationId),
            )
            .and_where(same(
                identity::Column::ScopeKey,
                github_observations::Column::ScopeKey,
            ))
            .and_where(github_generation_evidence())
            .and_where(generation_current(cutoff_ms))
            .to_owned();
        let orphans = github_observations::Entity::find()
            .select_only()
            .column(github_observations::Column::Id)
            .filter(github_observations::Column::JobRecordId.is_null())
            .filter(github_observations::Column::ObservedAt.lt(cutoff_ms))
            .filter(Expr::exists(referenced).not())
            .order_by_asc(github_observations::Column::ObservedAt)
            .order_by_asc(github_observations::Column::Id)
            .limit(BATCH)
            .into_query();
        github_observations::Entity::delete_many()
            .filter(github_observations::Column::Id.in_subquery(orphans))
            .exec(&tx)
            .await?;
        let forgejo = forgejo_jobs::Entity::find()
            .select_only()
            .column(forgejo_jobs::Column::Id)
            .filter(forgejo_jobs::Column::UpdatedAt.lt(cutoff_ms))
            .order_by_asc(forgejo_jobs::Column::UpdatedAt)
            .order_by_asc(forgejo_jobs::Column::Id)
            .limit(BATCH)
            .into_tuple::<String>()
            .all(&tx)
            .await?;
        for id in forgejo {
            forgejo_observations::Entity::delete_many()
                .filter(forgejo_observations::Column::JobRecordId.eq(id.as_str()))
                .exec(&tx)
                .await?;
            forgejo_jobs::Entity::delete_by_id(id).exec(&tx).await?;
        }
        let idle_forgejo = forgejo_polls::Entity::find()
            .select_only()
            .column(forgejo_polls::Column::ScopeKey)
            .filter(forgejo_polls::Column::AttemptedAt.lt(cutoff_ms))
            .filter(
                Expr::exists(
                    probe(forgejo_jobs::Entity)
                        .and_where(same(
                            forgejo_jobs::Column::ScopeKey,
                            forgejo_polls::Column::ScopeKey,
                        ))
                        .to_owned(),
                )
                .not(),
            )
            .limit(BATCH)
            .into_query();
        forgejo_polls::Entity::delete_many()
            .filter(forgejo_polls::Column::ScopeKey.in_subquery(idle_forgejo))
            .exec(&tx)
            .await?;
        // A Verified link leaves only with its job, and never while the linked
        // Generation or its retained Operation Logs are still current.
        let linked = probe(gitlab_links::Entity)
            .inner_join(
                generations::Entity,
                same(generations::Column::Id, gitlab_links::Column::GenerationId),
            )
            .and_where(same(
                gitlab_links::Column::JobRecordId,
                gitlab_jobs::Column::Id,
            ))
            .and_where(generation_current(cutoff_ms))
            .to_owned();
        let gitlab = gitlab_jobs::Entity::find()
            .select_only()
            .column(gitlab_jobs::Column::Id)
            .filter(gitlab_jobs::Column::UpdatedAt.lt(cutoff_ms))
            .filter(Expr::exists(linked).not())
            .order_by_asc(gitlab_jobs::Column::UpdatedAt)
            .order_by_asc(gitlab_jobs::Column::Id)
            .limit(BATCH)
            .into_tuple::<String>()
            .all(&tx)
            .await?;
        for id in gitlab {
            // Links go last: their trigger rejects deletes while the job exists.
            gitlab_observations::Entity::delete_many()
                .filter(gitlab_observations::Column::JobRecordId.eq(id.as_str()))
                .exec(&tx)
                .await?;
            gitlab_jobs::Entity::delete_by_id(id.as_str())
                .exec(&tx)
                .await?;
            gitlab_links::Entity::delete_many()
                .filter(gitlab_links::Column::JobRecordId.eq(id))
                .exec(&tx)
                .await?;
        }
        let idle_gitlab = gitlab_polls::Entity::find()
            .select_only()
            .column(gitlab_polls::Column::ScopeKey)
            .filter(gitlab_polls::Column::EnrichmentAttemptedAt.lt(cutoff_ms))
            .filter(
                Expr::exists(
                    probe(gitlab_jobs::Entity)
                        .and_where(same(
                            gitlab_jobs::Column::ScopeKey,
                            gitlab_polls::Column::ScopeKey,
                        ))
                        .to_owned(),
                )
                .not(),
            )
            .limit(BATCH)
            .into_query();
        gitlab_polls::Entity::delete_many()
            .filter(gitlab_polls::Column::ScopeKey.in_subquery(idle_gitlab))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
