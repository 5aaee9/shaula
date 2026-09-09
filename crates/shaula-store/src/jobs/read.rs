//! Read-only allowlisted DTOs: workspace, Terraform inputs, state and auth never escape.
use sea_orm::{QueryResult, TransactionTrait};
use shaula_core::jobs::{
    AssociationStatus, GenerationDetail, GenerationSummary, GenerationsPage, GenerationsQuery,
    JobDetail, JobObservation, JobSummary, JobsPage, JobsQuery, JobsReadError, JobsReadPort,
};

use super::{decode, query::PageQuery, rows};
use crate::{Store, StoreResult};

const GENERATIONS: &str = "SELECT g.id,g.fleet_key,g.runner_name,g.generation_name,g.state,g.subphase,
    g.template_profile_key,g.template_revision,g.created_at,g.updated_at,i.fleet_incarnation,
    COALESCE(i.github_runner_id,g.github_runner_id) AS github_runner_id,
    CASE WHEN EXISTS(SELECT 1 FROM workflow_job_observations o WHERE o.scope_key=i.scope_key
        AND o.runner_id=i.github_runner_id AND json_extract(o.data_json,'$.association_status')='ambiguous') THEN 'ambiguous'
    WHEN EXISTS(SELECT 1 FROM workflow_job_observations o
        WHERE json_extract(o.data_json,'$.generation_id')=g.id
        AND json_extract(o.data_json,'$.association_status')='verified') THEN 'verified'
    ELSE 'unverified' END AS association_status
    FROM runner_generations g LEFT JOIN workflow_generation_identity i ON i.generation_id=g.id";

#[async_trait::async_trait]
impl JobsReadPort for Store {
    async fn list_jobs(&self, query: JobsQuery) -> Result<JobsPage, JobsReadError> {
        let page = PageQuery::jobs(&query)?;
        let found = rows(
            self.connection(),
            &page.sql("SELECT summary_json,created_at,id FROM workflow_jobs"),
            page.values.clone(),
        )
        .await
        .map_err(unavailable)?;
        let mut items: Vec<JobSummary> = found
            .iter()
            .map(job_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        let more = items.len() > page.limit;
        items.truncate(page.limit);
        let next_cursor = if more {
            items
                .last()
                .map(|item| page.next(item.created_at, &item.id))
                .transpose()?
        } else {
            None
        };
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
        let found = rows(
            &tx,
            "SELECT summary_json FROM workflow_jobs WHERE id=?",
            vec![id.into()],
        )
        .await
        .map_err(unavailable)?;
        let Some(row) = found.first() else {
            return Ok(None);
        };
        let job = job_summary(row).map_err(unavailable)?;
        let found = rows(&tx, "SELECT data_json FROM workflow_job_observations WHERE job_record_id=? ORDER BY observed_at DESC,id DESC LIMIT 1001",
            vec![id.into()]).await.map_err(unavailable)?;
        let mut observations: Vec<JobObservation> = found
            .iter()
            .map(|r| decode(&r.try_get::<String>("", "data_json")?))
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        let observations_truncated = observations.len() > 1000;
        observations.truncate(1000);
        let sql = format!(
            "SELECT * FROM ({GENERATIONS}) WHERE id IN (SELECT DISTINCT json_extract(data_json,'$.generation_id') FROM workflow_job_observations WHERE job_record_id=?) ORDER BY created_at,id LIMIT 200"
        );
        let generations = rows(&tx, &sql, vec![id.into()])
            .await
            .map_err(unavailable)?
            .iter()
            .map(generation_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        Ok(Some(JobDetail {
            job,
            observations,
            observations_truncated,
            generations,
        }))
    }

    async fn list_generations(
        &self,
        query: GenerationsQuery,
    ) -> Result<GenerationsPage, JobsReadError> {
        let page = PageQuery::generations(&query)?;
        let select = format!("SELECT * FROM ({GENERATIONS})");
        let found = rows(self.connection(), &page.sql(&select), page.values.clone())
            .await
            .map_err(unavailable)?;
        let mut items: Vec<GenerationSummary> = found
            .iter()
            .map(generation_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        let more = items.len() > page.limit;
        items.truncate(page.limit);
        let next_cursor = if more {
            items
                .last()
                .map(|item| page.next(item.created_at, &item.id))
                .transpose()?
        } else {
            None
        };
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
        let sql = format!("SELECT * FROM ({GENERATIONS}) WHERE id=?");
        let found = rows(&tx, &sql, vec![id.into()])
            .await
            .map_err(unavailable)?;
        let Some(row) = found.first() else {
            return Ok(None);
        };
        let generation = generation_summary(row).map_err(unavailable)?;
        let found = rows(&tx, "SELECT j.summary_json FROM workflow_jobs j WHERE j.id IN (
            SELECT o.job_record_id FROM workflow_job_observations o JOIN workflow_generation_identity i
            ON i.scope_key=o.scope_key AND i.github_runner_id=o.runner_id WHERE i.generation_id=?
            AND (json_extract(o.data_json,'$.generation_id')=? OR json_extract(o.data_json,'$.association_status')='ambiguous'))
            ORDER BY j.created_at DESC,j.id DESC LIMIT 200", vec![id.into(), id.into()]).await.map_err(unavailable)?;
        let jobs = found
            .iter()
            .map(job_summary)
            .collect::<StoreResult<_>>()
            .map_err(unavailable)?;
        Ok(Some(GenerationDetail { generation, jobs }))
    }
}

fn job_summary(row: &QueryResult) -> StoreResult<JobSummary> {
    decode(&row.try_get::<String>("", "summary_json")?)
}

fn generation_summary(row: &QueryResult) -> StoreResult<GenerationSummary> {
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
