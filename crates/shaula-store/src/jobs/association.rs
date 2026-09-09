//! Exact numeric runner evidence within a captured target/Scale Set scope.
use sea_orm::ConnectionTrait;
use shaula_core::jobs::{project_observations, AssociationStatus, JobObservation, JobSummary};

use super::{decode, encode, execute, rows};
use crate::StoreResult;

pub(super) async fn refresh_runner_jobs<C: ConnectionTrait>(
    db: &C,
    scope: &str,
    runner: i64,
) -> StoreResult<()> {
    for row in rows(
        db,
        "SELECT DISTINCT job_record_id FROM workflow_job_observations
        WHERE scope_key=? AND runner_id=? AND job_record_id IS NOT NULL",
        vec![scope.into(), runner.into()],
    )
    .await?
    {
        refresh_job(db, &row.try_get::<String>("", "job_record_id")?).await?;
    }
    Ok(())
}

pub(super) async fn refresh_job<C: ConnectionTrait>(db: &C, id: &str) -> StoreResult<()> {
    let jobs = rows(
        db,
        "SELECT scope_key,summary_json FROM workflow_jobs WHERE id=?",
        vec![id.into()],
    )
    .await?;
    let Some(row) = jobs.first() else {
        return Ok(());
    };
    let scope: String = row.try_get("", "scope_key")?;
    let mut summary: JobSummary = decode(&row.try_get::<String>("", "summary_json")?)?;
    let mut observations = Vec::new();
    for row in rows(
        db,
        "SELECT data_json FROM workflow_job_observations WHERE job_record_id=?",
        vec![id.into()],
    )
    .await?
    {
        let mut observation: JobObservation = decode(&row.try_get::<String>("", "data_json")?)?;
        associate(db, &scope, &mut observation).await?;
        if request_conflicts(db, &scope, observation.runner_request_id).await? {
            observation.generation_id = None;
            observation.association_status = AssociationStatus::Ambiguous;
        }
        execute(
            db,
            "UPDATE workflow_job_observations SET data_json=? WHERE id=?",
            vec![encode(&observation)?.into(), observation.id.clone().into()],
        )
        .await?;
        observations.push(observation);
    }
    let projection = project_observations(&observations);
    if projection.association_status == AssociationStatus::Ambiguous {
        for observation in &mut observations {
            observation.generation_id = None;
            observation.association_status = AssociationStatus::Ambiguous;
            execute(
                db,
                "UPDATE workflow_job_observations SET data_json=? WHERE id=?",
                vec![encode(observation)?.into(), observation.id.clone().into()],
            )
            .await?;
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
    execute(db, "UPDATE workflow_jobs SET summary_json=?,status=?,repository=?,job_name=?,updated_at=? WHERE id=?",
        vec![encode(&summary)?.into(), summary.observed_status.as_str().into(), repository.into(),
            summary.metadata.job_display_name.into(), summary.updated_at.into(), id.into()]).await
}

async fn request_conflicts<C: ConnectionTrait>(
    db: &C,
    scope: &str,
    request: i64,
) -> StoreResult<bool> {
    if request <= 0 {
        return Ok(false);
    }
    let evidence = rows(
        db,
        "SELECT COUNT(DISTINCT NULLIF(protocol_job_id,'')) AS identities,
        SUM(CASE WHEN protocol_job_id='' THEN 1 ELSE 0 END) AS unresolved
        FROM workflow_job_observations WHERE scope_key=? AND runner_request_id=?",
        vec![scope.into(), request.into()],
    )
    .await?;
    let Some(row) = evidence.first() else {
        return Ok(false);
    };
    Ok(row.try_get::<i64>("", "identities")? > 1
        && row.try_get::<Option<i64>>("", "unresolved")?.unwrap_or(0) > 0)
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
    let candidates = rows(db, "SELECT i.generation_id,g.runner_name FROM workflow_generation_identity i
        JOIN runner_generations g ON g.id=i.generation_id WHERE i.scope_key=? AND i.github_runner_id=?",
        vec![scope.into(), runner.into()]).await?;
    if candidates.is_empty() {
        return Ok(());
    }
    let distinct_jobs = rows(
        db,
        "SELECT DISTINCT protocol_job_id FROM workflow_job_observations
        WHERE scope_key=? AND runner_id=? AND protocol_job_id<>''",
        vec![scope.into(), runner.into()],
    )
    .await?;
    let name: String = candidates[0].try_get("", "runner_name")?;
    if candidates.len() != 1
        || distinct_jobs.len() > 1
        || observation.runner_name.as_ref().is_some_and(|n| n != &name)
    {
        observation.association_status = AssociationStatus::Ambiguous;
        return Ok(());
    }
    observation.generation_id = Some(candidates[0].try_get("", "generation_id")?);
    observation.association_status = AssociationStatus::Verified;
    Ok(())
}
