//! Exact numeric runner evidence within a captured target/Scale Set scope.
use sea_orm::{
    sea_query::{Expr, Func},
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::jobs::{project_observations, AssociationStatus, JobObservation, JobSummary};

use super::{decode, encode};
use crate::entities::{
    jobs::{
        workflow_generation_identity as identity, workflow_job_observations as observations,
        workflow_jobs as jobs,
    },
    lifecycle::runner_generations as generations,
};
use crate::query_ext::{same, JoinOn};
use crate::StoreResult;

pub(super) async fn refresh_runner_jobs<C: ConnectionTrait>(
    db: &C,
    scope: &str,
    runner: i64,
) -> StoreResult<()> {
    let ids = observations::Entity::find()
        .select_only()
        .column(observations::Column::JobRecordId)
        .distinct()
        .filter(observations::Column::ScopeKey.eq(scope))
        .filter(observations::Column::RunnerId.eq(runner))
        .filter(observations::Column::JobRecordId.is_not_null())
        .into_tuple::<String>()
        .all(db)
        .await?;
    for id in ids {
        refresh_job(db, &id).await?;
    }
    Ok(())
}

async fn save_observation<C: ConnectionTrait>(
    db: &C,
    observation: &JobObservation,
) -> StoreResult<()> {
    observations::Entity::update_many()
        .col_expr(
            observations::Column::DataJson,
            Expr::value(encode(observation)?),
        )
        .filter(observations::Column::Id.eq(observation.id.as_str()))
        .exec(db)
        .await?;
    Ok(())
}

pub(super) async fn refresh_job<C: ConnectionTrait>(db: &C, id: &str) -> StoreResult<()> {
    let job = jobs::Entity::find_by_id(id)
        .select_only()
        .columns([jobs::Column::ScopeKey, jobs::Column::SummaryJson])
        .into_tuple::<(String, String)>()
        .one(db)
        .await?;
    let Some((scope, summary_json)) = job else {
        return Ok(());
    };
    let mut summary: JobSummary = decode(&summary_json)?;
    let mut observations = Vec::new();
    for data_json in observations::Entity::find()
        .select_only()
        .column(observations::Column::DataJson)
        .filter(observations::Column::JobRecordId.eq(id))
        .into_tuple::<String>()
        .all(db)
        .await?
    {
        let mut observation: JobObservation = decode(&data_json)?;
        associate(db, &scope, &mut observation).await?;
        if request_conflicts(db, &scope, observation.runner_request_id).await? {
            observation.generation_id = None;
            observation.association_status = AssociationStatus::Ambiguous;
        }
        save_observation(db, &observation).await?;
        observations.push(observation);
    }
    let projection = project_observations(&observations);
    if projection.association_status == AssociationStatus::Ambiguous {
        for observation in &mut observations {
            observation.generation_id = None;
            observation.association_status = AssociationStatus::Ambiguous;
            save_observation(db, observation).await?;
        }
    }
    summary.metadata = projection.metadata;
    summary.observed_status = projection.observed_status;
    summary.reported_result = projection.reported_result;
    summary.association_status = projection.association_status;
    summary.github_run_url = summary.metadata.github_run_url();
    if let Some(updated) = observations.iter().map(|o| o.observed_at).max() {
        summary.updated_at = updated;
    }
    let repository = summary
        .metadata
        .owner_name
        .as_ref()
        .zip(summary.metadata.repository_name.as_ref())
        .map(|(owner, repo)| format!("{owner}/{repo}"));
    jobs::Entity::update_many()
        .col_expr(jobs::Column::SummaryJson, Expr::value(encode(&summary)?))
        .col_expr(
            jobs::Column::Status,
            Expr::value(summary.observed_status.as_str()),
        )
        .col_expr(jobs::Column::Repository, Expr::value(repository))
        .col_expr(
            jobs::Column::JobName,
            Expr::value(summary.metadata.job_display_name),
        )
        .col_expr(jobs::Column::UpdatedAt, Expr::value(summary.updated_at))
        .filter(jobs::Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(())
}

async fn request_conflicts<C: ConnectionTrait>(
    db: &C,
    scope: &str,
    request: i64,
) -> StoreResult<bool> {
    if request <= 0 {
        return Ok(false);
    }
    let protocol = Expr::col(observations::Column::ProtocolJobId);
    let evidence = observations::Entity::find()
        .select_only()
        .expr(Func::count_distinct(
            Func::cust(sea_orm::sea_query::Alias::new("NULLIF"))
                .arg(protocol.clone())
                .arg(""),
        ))
        .expr(Func::sum(Expr::case(protocol.eq(""), 1).finally(0)))
        .filter(observations::Column::ScopeKey.eq(scope))
        .filter(observations::Column::RunnerRequestId.eq(request))
        .into_tuple::<(i64, Option<i64>)>()
        .one(db)
        .await?;
    let Some((identities, unresolved)) = evidence else {
        return Ok(false);
    };
    Ok(identities > 1 && unresolved.unwrap_or(0) > 0)
}

async fn associate<C: ConnectionTrait>(
    db: &C,
    scope: &str,
    observation: &mut JobObservation,
) -> StoreResult<()> {
    observation.generation_id = None;
    observation.association_status = AssociationStatus::Unverified;
    let Some(runner) = observation.runner_id else {
        return Ok(());
    };
    let candidates = identity::Entity::find()
        .select_only()
        .column(identity::Column::GenerationId)
        .column(generations::Column::RunnerName)
        .inner_join_on(
            generations::Entity,
            same(generations::Column::Id, identity::Column::GenerationId),
        )
        .filter(identity::Column::ScopeKey.eq(scope))
        .filter(identity::Column::GithubRunnerId.eq(runner))
        .into_tuple::<(String, String)>()
        .all(db)
        .await?;
    let Some((generation, name)) = candidates.first() else {
        return Ok(());
    };
    let distinct_jobs = observations::Entity::find()
        .select_only()
        .column(observations::Column::ProtocolJobId)
        .distinct()
        .filter(observations::Column::ScopeKey.eq(scope))
        .filter(observations::Column::RunnerId.eq(runner))
        .filter(observations::Column::ProtocolJobId.ne(""))
        .into_tuple::<String>()
        .all(db)
        .await?;
    if candidates.len() != 1
        || distinct_jobs.len() > 1
        || observation.runner_name.as_ref().is_some_and(|n| n != name)
    {
        observation.association_status = AssociationStatus::Ambiguous;
        return Ok(());
    }
    observation.generation_id = Some(generation.clone());
    observation.association_status = AssociationStatus::Verified;
    Ok(())
}
