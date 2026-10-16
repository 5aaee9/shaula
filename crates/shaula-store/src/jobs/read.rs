//! Read-only allowlisted DTOs: workspace, Terraform inputs, state and auth never escape.
use sea_orm::{
    sea_query::{Alias, Expr, Order, Query},
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QueryResult, QuerySelect, TransactionTrait,
};
use shaula_core::jobs::{
    AssociationStatus, GenerationDetail, GenerationSummary, GenerationsPage, GenerationsQuery,
    JobDetail, JobObservation, JobSummary, JobsPage, JobsQuery, JobsReadError, JobsReadPort,
};

use super::{decode, query::PageQuery, select_rows, views};
use crate::entities::jobs::{
    workflow_generation_identity as identity, workflow_job_observations as observations,
    workflow_jobs as github_jobs,
};
use crate::query_ext::{col, json_extract, same};
use crate::{Store, StoreResult};

#[async_trait::async_trait]
impl JobsReadPort for Store {
    async fn list_jobs(&self, query: JobsQuery) -> Result<JobsPage, JobsReadError> {
        let page = PageQuery::jobs(&query)?;
        let found = select_rows(self.connection(), &page.select(views::jobs()))
            .await
            .map_err(unavailable)?;
        let items: Vec<JobSummary> = found
            .iter()
            .map(job_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        let (items, next_cursor) = page.finish(items, |item| (item.created_at, &item.id))?;
        Ok(JobsPage { items, next_cursor })
    }

    async fn get_job(&self, id: &str) -> Result<Option<JobDetail>, JobsReadError> {
        if id.len() > 128 {
            return Ok(None);
        }
        let tx = self
            .connection()
            .begin()
            .await
            .map_err(|_| JobsReadError::Unavailable)?;
        let job = views::from_view(views::jobs())
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned();
        let found = select_rows(&tx, &job).await.map_err(unavailable)?;
        let Some(row) = found.first() else {
            return Ok(None);
        };
        let job = job_summary(row).map_err(unavailable)?;
        if job.backend == shaula_core::fleet::FleetProviderKind::Forgejo {
            return super::forgejo_read::detail(&tx, job)
                .await
                .map(Some)
                .map_err(unavailable);
        }
        if job.backend == shaula_core::fleet::FleetProviderKind::Gitlab {
            return super::gitlab_read::detail(&tx, job)
                .await
                .map(Some)
                .map_err(unavailable);
        }
        let found = observations::Entity::find()
            .select_only()
            .column(observations::Column::DataJson)
            .filter(observations::Column::JobRecordId.eq(id))
            .order_by_desc(observations::Column::ObservedAt)
            .order_by_desc(observations::Column::Id)
            .limit(1001)
            .into_tuple::<String>()
            .all(&tx)
            .await
            .map_err(|_| JobsReadError::Unavailable)?;
        let mut observations: Vec<JobObservation> = found
            .iter()
            .map(|json| decode(json))
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        let observations_truncated = observations.len() > 1000;
        observations.truncate(1000);
        let associated = Query::select()
            .distinct()
            .expr(json_extract(
                observations::Column::DataJson,
                "$.generation_id",
            ))
            .from(observations::Entity)
            .and_where(Expr::col(observations::Column::JobRecordId).eq(id))
            .to_owned();
        let mut select = views::from_view(views::generations());
        select.and_where(Expr::col(Alias::new("id")).in_subquery(associated));
        views::keyset(&mut select, Order::Asc).limit(200);
        let generations = select_rows(&tx, &select)
            .await
            .map_err(unavailable)?
            .iter()
            .map(generation_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        Ok(Some(JobDetail {
            job,
            observations,
            forgejo_observations: Vec::new(),
            gitlab_observations: Vec::new(),
            observations_truncated,
            generations,
        }))
    }

    async fn list_generations(
        &self,
        query: GenerationsQuery,
    ) -> Result<GenerationsPage, JobsReadError> {
        let page = PageQuery::generations(&query)?;
        let found = select_rows(self.connection(), &page.select(views::generations()))
            .await
            .map_err(unavailable)?;
        let items: Vec<GenerationSummary> = found
            .iter()
            .map(generation_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        let (items, next_cursor) = page.finish(items, |item| (item.created_at, &item.id))?;
        Ok(GenerationsPage { items, next_cursor })
    }

    async fn get_generation(&self, id: &str) -> Result<Option<GenerationDetail>, JobsReadError> {
        if id.len() > 128 {
            return Ok(None);
        }
        let tx = self
            .connection()
            .begin()
            .await
            .map_err(|_| JobsReadError::Unavailable)?;
        let generation = views::from_view(views::generations())
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned();
        let found = select_rows(&tx, &generation).await.map_err(unavailable)?;
        let Some(row) = found.first() else {
            return Ok(None);
        };
        let generation = generation_summary(row).map_err(unavailable)?;
        let observed = |path| {
            Expr::expr(json_extract(
                observations::Column::DataJson.as_column_ref(),
                path,
            ))
        };
        let ran = Query::select()
            .column(observations::Column::JobRecordId.as_column_ref())
            .from(observations::Entity)
            .inner_join(
                identity::Entity,
                same(identity::Column::ScopeKey, observations::Column::ScopeKey).and(same(
                    identity::Column::GithubRunnerId,
                    observations::Column::RunnerId,
                )),
            )
            .and_where(col(identity::Column::GenerationId).eq(id))
            .and_where(
                observed("$.generation_id")
                    .eq(id)
                    .or(observed("$.association_status").eq("ambiguous")),
            )
            .to_owned();
        let jobs_run = Query::select()
            .column(github_jobs::Column::SummaryJson.as_column_ref())
            .expr_as(views::runner_retired(), Alias::new("runner_retired"))
            .from(github_jobs::Entity)
            .and_where(col(github_jobs::Column::Id).in_subquery(ran))
            .order_by(github_jobs::Column::CreatedAt.as_column_ref(), Order::Desc)
            .order_by(github_jobs::Column::Id.as_column_ref(), Order::Desc)
            .limit(200)
            .to_owned();
        let found = select_rows(&tx, &jobs_run).await.map_err(unavailable)?;
        let mut jobs: Vec<JobSummary> = found
            .iter()
            .map(job_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        if generation.gitlab_runner_id.is_some() {
            jobs.extend(
                super::gitlab_read::generation_jobs(&tx, id)
                    .await
                    .map_err(unavailable)?,
            );
        }
        Ok(Some(GenerationDetail { generation, jobs }))
    }
}

fn job_summary(row: &QueryResult) -> StoreResult<JobSummary> {
    let mut job: JobSummary = decode(&row.try_get::<String>("", "summary_json")?)?;
    super::forgejo_read::freshness(&mut job, row);
    super::gitlab_read::freshness(&mut job);
    if row
        .try_get::<Option<i64>>("", "runner_retired")?
        .is_some_and(|flag| flag != 0)
    {
        job.observed_status = shaula_core::jobs::ObservedStatus::Unknown;
        job.freshness = "stale".into();
    }
    Ok(job)
}

pub(super) fn generation_summary(row: &QueryResult) -> StoreResult<GenerationSummary> {
    let association: String = row.try_get("", "association_status")?;
    let association_status = match association.as_str() {
        "verified" => AssociationStatus::Verified,
        "ambiguous" => AssociationStatus::Ambiguous,
        _ => AssociationStatus::Unverified,
    };
    Ok(GenerationSummary {
        id: row.try_get("", "id")?,
        fleet_key: row.try_get("", "fleet_key")?,
        fleet_incarnation: row.try_get("", "fleet_incarnation")?,
        runner_name: row.try_get("", "runner_name")?,
        generation_name: row.try_get("", "generation_name")?,
        github_runner_id: row.try_get("", "github_runner_id")?,
        forgejo_runner_id: row
            .try_get::<Option<i64>>("", "forgejo_runner_id")?
            .map(|id| id.to_string()),
        gitlab_runner_id: row
            .try_get::<Option<i64>>("", "gitlab_runner_id")?
            .map(|id| id.to_string()),
        state: row.try_get("", "state")?,
        subphase: row.try_get("", "subphase")?,
        template_profile_key: row.try_get("", "template_profile_key")?,
        template_revision: row.try_get("", "template_revision")?,
        association_status,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
    })
}

fn unavailable(_: crate::StoreError) -> JobsReadError {
    JobsReadError::Unavailable
}
