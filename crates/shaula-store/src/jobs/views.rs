//! Read-model projections over the GitHub, Forgejo and GitLab job tables
//! and the Generation ledger. Callers select from them as subqueries.
use sea_orm::{
    sea_query::{
        Alias, Asterisk, Expr, Func, Keyword, Order, Query, SelectStatement, SimpleExpr, UnionType,
    },
    ColumnTrait,
};

use crate::entities::{
    gitlab::gitlab_runner_identities as gitlab_identities,
    jobs::{
        forgejo_job_polls as forgejo_polls, forgejo_workflow_jobs as forgejo_jobs,
        gitlab_job_generations as gitlab_links, gitlab_workflow_jobs as gitlab_jobs,
        workflow_generation_identity as identity, workflow_job_observations as observations,
        workflow_jobs as github_jobs,
    },
    lifecycle::{
        forgejo_runner_identities as forgejo_identities, runner_generations as generations,
    },
};
use crate::query_ext::{col, json_extract, probe, same};

/// `json_extract` on the observation row in scope.
fn observed(path: &'static str) -> Expr {
    Expr::expr(json_extract(
        observations::Column::DataJson.as_column_ref(),
        path,
    ))
}

fn null() -> SimpleExpr {
    SimpleExpr::Keyword(Keyword::Null)
}

/// A GitHub job last observed `running` whose every verified Generation is
/// Destroyed can no longer be running there; its Completed was never observed
/// (e.g. lost across a listener restart). Spec 0019 §2.2: report the status as
/// unknown/stale and keep the retained observations, never invent a completion.
pub(super) fn runner_retired() -> SimpleExpr {
    let verified = |state: SimpleExpr| {
        probe(observations::Entity)
            .inner_join(
                generations::Entity,
                col(generations::Column::Id).eq(observed("$.generation_id")),
            )
            .and_where(same(
                observations::Column::JobRecordId,
                github_jobs::Column::Id,
            ))
            .and_where(observed("$.association_status").eq("verified"))
            .and_where(state)
            .to_owned()
    };
    col(github_jobs::Column::Status)
        .eq("running")
        .and(Expr::exists(verified(
            col(generations::Column::State).eq("Destroyed"),
        )))
        .and(Expr::exists(verified(col(generations::Column::State).ne("Destroyed"))).not())
}

/// Every job record with a common column set; GitHub `running` jobs on
/// retired runners read as `unknown`.
pub(super) fn jobs() -> SelectStatement {
    let common = |select: &mut SelectStatement, id, fleet, summary| {
        select.column(id).column(fleet).column(summary);
    };
    let mut github = Query::select();
    common(
        &mut github,
        github_jobs::Column::Id.as_column_ref(),
        github_jobs::Column::FleetKey.as_column_ref(),
        github_jobs::Column::SummaryJson.as_column_ref(),
    );
    github
        .expr_as(
            Expr::case(runner_retired(), "unknown").finally(col(github_jobs::Column::Status)),
            Alias::new("status"),
        )
        .columns([
            github_jobs::Column::Repository.as_column_ref(),
            github_jobs::Column::JobName.as_column_ref(),
            github_jobs::Column::CreatedAt.as_column_ref(),
            github_jobs::Column::UpdatedAt.as_column_ref(),
        ])
        .expr_as(null(), Alias::new("poll_time"))
        .expr_as(null(), Alias::new("poll_failed"))
        .expr_as(runner_retired(), Alias::new("runner_retired"))
        .from(github_jobs::Entity);
    let mut forgejo = Query::select();
    common(
        &mut forgejo,
        forgejo_jobs::Column::Id.as_column_ref(),
        forgejo_jobs::Column::FleetKey.as_column_ref(),
        forgejo_jobs::Column::SummaryJson.as_column_ref(),
    );
    forgejo
        .columns([
            forgejo_jobs::Column::Status.as_column_ref(),
            forgejo_jobs::Column::Repository.as_column_ref(),
            forgejo_jobs::Column::JobName.as_column_ref(),
            forgejo_jobs::Column::CreatedAt.as_column_ref(),
            forgejo_jobs::Column::UpdatedAt.as_column_ref(),
        ])
        .expr_as(
            col(forgejo_polls::Column::ObservedAt),
            Alias::new("poll_time"),
        )
        .expr_as(
            col(forgejo_polls::Column::Failed),
            Alias::new("poll_failed"),
        )
        .expr_as(Expr::val(0), Alias::new("runner_retired"))
        .from(forgejo_jobs::Entity)
        .left_join(
            forgejo_polls::Entity,
            same(
                forgejo_polls::Column::ScopeKey,
                forgejo_jobs::Column::ScopeKey,
            ),
        );
    let mut gitlab = Query::select();
    common(
        &mut gitlab,
        gitlab_jobs::Column::Id.as_column_ref(),
        gitlab_jobs::Column::FleetKey.as_column_ref(),
        gitlab_jobs::Column::SummaryJson.as_column_ref(),
    );
    gitlab
        .columns([
            gitlab_jobs::Column::Status.as_column_ref(),
            gitlab_jobs::Column::Repository.as_column_ref(),
            gitlab_jobs::Column::JobName.as_column_ref(),
            gitlab_jobs::Column::CreatedAt.as_column_ref(),
            gitlab_jobs::Column::UpdatedAt.as_column_ref(),
        ])
        .expr_as(null(), Alias::new("poll_time"))
        .expr_as(null(), Alias::new("poll_failed"))
        .expr_as(Expr::val(0), Alias::new("runner_retired"))
        .from(gitlab_jobs::Entity);
    github
        .union(UnionType::All, forgejo)
        .union(UnionType::All, gitlab)
        .to_owned()
}

/// Generation summaries with backend runner identities and the strongest
/// job association evidence: ambiguous, then verified, else unverified.
pub(super) fn generations() -> SelectStatement {
    let ambiguous = probe(observations::Entity)
        .and_where(same(
            observations::Column::ScopeKey,
            identity::Column::ScopeKey,
        ))
        .and_where(same(
            observations::Column::RunnerId,
            identity::Column::GithubRunnerId,
        ))
        .and_where(observed("$.association_status").eq("ambiguous"))
        .to_owned();
    let verified = probe(observations::Entity)
        .and_where(observed("$.generation_id").equals(generations::Column::Id.as_column_ref()))
        .and_where(observed("$.association_status").eq("verified"))
        .to_owned();
    let linked = probe(gitlab_links::Entity)
        .and_where(same(
            gitlab_links::Column::GenerationId,
            generations::Column::Id,
        ))
        .to_owned();
    Query::select()
        .columns(
            [
                generations::Column::Id,
                generations::Column::FleetKey,
                generations::Column::RunnerName,
                generations::Column::GenerationName,
                generations::Column::State,
                generations::Column::Subphase,
                generations::Column::TemplateProfileKey,
                generations::Column::TemplateRevision,
                generations::Column::CreatedAt,
                generations::Column::UpdatedAt,
            ]
            .map(|column| column.as_column_ref()),
        )
        .column(identity::Column::FleetIncarnation.as_column_ref())
        .expr_as(
            col(forgejo_identities::Column::RunnerId),
            Alias::new("forgejo_runner_id"),
        )
        .expr_as(
            col(gitlab_identities::Column::RunnerId),
            Alias::new("gitlab_runner_id"),
        )
        .expr_as(
            Func::coalesce([
                col(identity::Column::GithubRunnerId).into(),
                col(generations::Column::GithubRunnerId).into(),
            ]),
            Alias::new("github_runner_id"),
        )
        .expr_as(
            Expr::case(Expr::exists(ambiguous), "ambiguous")
                .case(Expr::exists(verified), "verified")
                .case(Expr::exists(linked), "verified")
                .finally("unverified"),
            Alias::new("association_status"),
        )
        .from(generations::Entity)
        .left_join(
            identity::Entity,
            same(identity::Column::GenerationId, generations::Column::Id),
        )
        .left_join(
            forgejo_identities::Entity,
            same(
                forgejo_identities::Column::GenerationId,
                generations::Column::Id,
            ),
        )
        .left_join(
            gitlab_identities::Entity,
            same(
                gitlab_identities::Column::GenerationId,
                generations::Column::Id,
            ),
        )
        .to_owned()
}

/// `SELECT * FROM (view)`, with view columns addressed unqualified.
pub(super) fn from_view(view: SelectStatement) -> SelectStatement {
    Query::select()
        .column(Asterisk)
        .from_subquery(view, Alias::new("view"))
        .to_owned()
}

/// The `created_at, id` keyset order shared by every list.
pub(super) fn keyset(select: &mut SelectStatement, order: Order) -> &mut SelectStatement {
    select
        .order_by(Alias::new("created_at"), order.clone())
        .order_by(Alias::new("id"), order)
}
