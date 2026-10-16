//! Snapshot projection for Forgejo, isolated from GitHub listener observations.
use super::{decode, encode, forgejo_projection};
use crate::entities::jobs::{forgejo_job_polls as polls, forgejo_workflow_jobs as jobs_table};
use crate::{
    registry_impl::{core_err, SqliteControlPlane},
    Store, StoreError, StoreResult,
};
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveValue::Set,
    ColumnTrait, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::{
    jobs::{ForgejoJobsStore, JobSummary, ObservedStatus},
    ports::forgejo::ForgejoJob,
    registry::FleetRuntimeGuard,
};
use std::collections::BTreeSet;

#[async_trait::async_trait]
impl ForgejoJobsStore for SqliteControlPlane {
    async fn forgejo_jobs_pending_results(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        now: i64,
    ) -> shaula_core::error::CoreResult<Vec<shaula_core::jobs::ForgejoJobLookup>> {
        self.store()
            .pending_forgejo_results(fleet, guard, now)
            .await
            .map_err(core_err)
    }

    async fn forgejo_job_result(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        lookup: &shaula_core::jobs::ForgejoJobLookup,
        result: &shaula_core::jobs::ForgejoTaskResult,
        now: i64,
    ) -> shaula_core::error::CoreResult<bool> {
        self.store()
            .apply_forgejo_result(fleet, guard, lookup, result, now)
            .await
            .map_err(core_err)
    }

    async fn forgejo_jobs_snapshot(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        jobs: Option<&[ForgejoJob]>,
        now: i64,
    ) -> shaula_core::error::CoreResult<bool> {
        self.store()
            .observe_forgejo_jobs(fleet, guard, jobs, now)
            .await
            .map_err(core_err)
    }
}

impl Store {
    async fn observe_forgejo_jobs(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        jobs: Option<&[ForgejoJob]>,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.begin().await?;
        let Some(scope) = super::forgejo_scope::current(&tx, fleet, guard).await? else {
            return Ok(false);
        };
        let target = scope.target;
        let scope = scope.key;
        let previous = polls::Entity::find_by_id(scope.as_str())
            .select_only()
            .column(polls::Column::AttemptedAt)
            .into_tuple::<i64>()
            .one(&tx)
            .await?;
        if previous.is_some_and(|previous| now < previous) {
            return Ok(false);
        }
        let Some(jobs) = jobs else {
            polls::Entity::insert(polls::ActiveModel {
                scope_key: Set(scope),
                observed_at: Set(0),
                attempted_at: Set(now),
                failed: Set(true),
                ..Default::default()
            })
            .on_conflict(
                OnConflict::column(polls::Column::ScopeKey)
                    .update_column(polls::Column::AttemptedAt)
                    .value(polls::Column::Failed, true)
                    .to_owned(),
            )
            .exec_without_returning(&tx)
            .await?;
            tx.commit().await?;
            return Ok(true);
        };
        validate(jobs)?;
        let mut present = BTreeSet::new();
        for job in jobs {
            let id = forgejo_projection::upsert(
                &tx,
                fleet,
                &guard.incarnation,
                &scope,
                &target,
                job,
                now,
            )
            .await?;
            present.insert(id);
        }
        for (id, summary_json) in jobs_table::Entity::find()
            .select_only()
            .columns([jobs_table::Column::Id, jobs_table::Column::SummaryJson])
            .filter(jobs_table::Column::ScopeKey.eq(scope.as_str()))
            .filter(jobs_table::Column::InSnapshot.eq(true))
            .into_tuple::<(String, String)>()
            .all(&tx)
            .await?
        {
            if present.contains(&id) {
                continue;
            }
            let mut summary: JobSummary = decode(&summary_json)?;
            let state = summary
                .forgejo
                .as_mut()
                .ok_or_else(|| StoreError::Corrupt("Forgejo job metadata missing".into()))?;
            state.in_snapshot = false;
            let task_id = state.task_id.clone();
            if state.result.is_none() {
                summary.observed_status = ObservedStatus::Unknown;
            }
            summary.updated_at = summary.updated_at.max(now);
            // Last listed time stays attributed to the actual job. Absence is
            // a separate retained observation, never a synthetic success/result.
            jobs_table::Entity::update_many()
                .col_expr(
                    jobs_table::Column::SummaryJson,
                    Expr::value(encode(&summary)?),
                )
                .col_expr(
                    jobs_table::Column::Status,
                    Expr::value(summary.observed_status.as_str()),
                )
                .col_expr(jobs_table::Column::InSnapshot, Expr::value(false))
                .col_expr(
                    jobs_table::Column::UpdatedAt,
                    Expr::value(summary.updated_at),
                )
                .filter(jobs_table::Column::Id.eq(id.as_str()))
                .exec(&tx)
                .await?;
            forgejo_projection::event(&tx, &id, None, &task_id, now).await?;
        }
        polls::Entity::insert(polls::ActiveModel {
            scope_key: Set(scope),
            observed_at: Set(now),
            attempted_at: Set(now),
            failed: Set(false),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(polls::Column::ScopeKey)
                .update_columns([polls::Column::ObservedAt, polls::Column::AttemptedAt])
                .value(polls::Column::Failed, false)
                .to_owned(),
        )
        .exec_without_returning(&tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
}

fn validate(jobs: &[ForgejoJob]) -> StoreResult<()> {
    shaula_core::forgejo::validate_job_identities(jobs)
        .map_err(|_| StoreError::Corrupt("invalid Forgejo job snapshot identities".into()))?;
    let invalid = jobs.iter().any(|job| {
        job.name.len() > 1024
            || job.status.len() > 64
            || job.status.chars().any(char::is_control)
            || job.runs_on.len() > 32
            || job
                .runs_on
                .iter()
                .any(|label| label.len() > 255 || label.chars().any(char::is_control))
    });
    if invalid {
        return Err(StoreError::Corrupt("invalid Forgejo job snapshot".into()));
    }
    Ok(())
}
