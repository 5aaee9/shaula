//! Scrape-time aggregates for the Prometheus endpoint. Only counts leave
//! the store, grouped by fleet and fixed status codes; rows, IDs and
//! payloads never do.

use sea_orm::{
    sea_query::{Expr, Func, SimpleExpr},
    EntityTrait, QuerySelect,
};
use shaula_core::error::CoreResult;
use shaula_core::ports::ledger_metrics::{GenerationCount, JobCount, LedgerMetricsPort};

use crate::entities::{
    jobs::{forgejo_workflow_jobs, gitlab_workflow_jobs, workflow_jobs},
    lifecycle::runner_generations as generations,
};
use crate::query_ext::json_extract;
use crate::registry_impl::core_err;
use crate::Store;

fn count_star() -> SimpleExpr {
    Func::count(Expr::val(1)).into()
}

macro_rules! job_counts {
    ($db:expr, $table:ident, $backend:literal) => {{
        let reported = || json_extract($table::Column::SummaryJson, "$.reported_result");
        let rows: Vec<(String, String, Option<String>, i64)> = $table::Entity::find()
            .select_only()
            .column($table::Column::FleetKey)
            .column($table::Column::Status)
            .expr(reported())
            .expr(count_star())
            .group_by($table::Column::FleetKey)
            .group_by($table::Column::Status)
            .group_by(reported())
            .into_tuple()
            .all($db)
            .await
            .map_err(|e| core_err(e.into()))?;
        rows.into_iter()
            .map(|(fleet_key, status, result, count)| JobCount {
                fleet_key,
                backend: $backend,
                status,
                result,
                count,
            })
    }};
}

#[async_trait::async_trait]
impl LedgerMetricsPort for Store {
    async fn generation_counts(&self) -> CoreResult<Vec<GenerationCount>> {
        let rows: Vec<(String, String, i64)> = generations::Entity::find()
            .select_only()
            .column(generations::Column::FleetKey)
            .column(generations::Column::State)
            .expr(count_star())
            .group_by(generations::Column::FleetKey)
            .group_by(generations::Column::State)
            .into_tuple()
            .all(self.connection())
            .await
            .map_err(|e| core_err(e.into()))?;
        Ok(rows
            .into_iter()
            .map(|(fleet_key, state, count)| GenerationCount {
                fleet_key,
                state,
                count,
            })
            .collect())
    }

    async fn job_counts(&self) -> CoreResult<Vec<JobCount>> {
        let db = self.connection();
        let mut counts: Vec<JobCount> = job_counts!(db, workflow_jobs, "github").collect();
        counts.extend(job_counts!(db, forgejo_workflow_jobs, "forgejo"));
        counts.extend(job_counts!(db, gitlab_workflow_jobs, "gitlab"));
        Ok(counts)
    }
}
