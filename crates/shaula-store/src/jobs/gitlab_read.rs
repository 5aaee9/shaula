//! Read-time freshness expires even if the daemon stopped reading; the
//! Generation list holds only append-only Verified links.
use super::{decode, read::generation_summary, select_rows, views};
use crate::entities::jobs::{
    gitlab_job_generations as links, gitlab_job_observations as observations,
    gitlab_workflow_jobs as jobs_table,
};
use crate::StoreResult;
use sea_orm::{
    sea_query::{Alias, Expr, Order, SelectStatement},
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
};
use shaula_core::jobs::{GitlabJobObservation, JobDetail, JobSummary};

pub(super) fn freshness(job: &mut JobSummary) {
    let Some(state) = &job.gitlab else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());
    job.freshness = if state.result.is_some() {
        "confirmed"
    } else if !state.listed {
        "not_listed"
    } else if now.is_some_and(|now| {
        now >= state.last_observed_at && now <= state.last_observed_at.saturating_add(30_000)
    }) {
        "fresh"
    } else {
        "stale"
    }
    .into();
}

pub(super) async fn detail<C: ConnectionTrait>(tx: &C, job: JobSummary) -> StoreResult<JobDetail> {
    let found = observations::Entity::find()
        .select_only()
        .column(observations::Column::DataJson)
        .filter(observations::Column::JobRecordId.eq(job.id.as_str()))
        .order_by_desc(observations::Column::ObservedAt)
        .order_by_desc(observations::Column::Id)
        .limit(1001)
        .into_tuple::<String>()
        .all(tx)
        .await?;
    let mut observations: Vec<GitlabJobObservation> = found
        .iter()
        .map(|json| decode(json))
        .collect::<StoreResult<_>>()?;
    let observations_truncated = observations.len() > 1000;
    observations.truncate(1000);
    let mut select = views::from_view(views::generations());
    select.and_where(Expr::col(Alias::new("id")).in_subquery(linked_generations(&job.id)));
    views::keyset(&mut select, Order::Asc).limit(200);
    let generations = select_rows(tx, &select)
        .await?
        .iter()
        .map(generation_summary)
        .collect::<StoreResult<_>>()?;
    Ok(JobDetail {
        job,
        observations: Vec::new(),
        forgejo_observations: Vec::new(),
        gitlab_observations: observations,
        observations_truncated,
        generations,
    })
}

/// Jobs a GitLab Generation's registration ran, by its Verified links.
pub(super) async fn generation_jobs<C: ConnectionTrait>(
    tx: &C,
    generation: &str,
) -> StoreResult<Vec<JobSummary>> {
    let linked = links::Entity::find()
        .select_only()
        .column(links::Column::JobRecordId)
        .filter(links::Column::GenerationId.eq(generation))
        .into_query();
    jobs_table::Entity::find()
        .select_only()
        .column(jobs_table::Column::SummaryJson)
        .filter(jobs_table::Column::Id.in_subquery(linked))
        .order_by_desc(jobs_table::Column::CreatedAt)
        .order_by_desc(jobs_table::Column::Id)
        .limit(200)
        .into_tuple::<String>()
        .all(tx)
        .await?
        .iter()
        .map(|json| {
            let mut job: JobSummary = decode(json)?;
            freshness(&mut job);
            Ok(job)
        })
        .collect()
}

/// Generations a GitLab job's append-only Verified links name.
fn linked_generations(job: &str) -> SelectStatement {
    links::Entity::find()
        .select_only()
        .column(links::Column::GenerationId)
        .filter(links::Column::JobRecordId.eq(job))
        .into_query()
}
