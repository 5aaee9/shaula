//! Claim-time projection for GitLab, isolated from GitHub and Forgejo jobs.
use super::{decode, gitlab_projection, gitlab_scope};
use crate::entities::{
    fleet::fleet_revisions,
    gitlab::{gitlab_job_associations as associations, gitlab_runner_identities as identities},
    jobs::{gitlab_job_generations as links, gitlab_workflow_jobs as jobs_table},
    lifecycle::runner_generations as generations,
};
use crate::query_ext::{same, JoinOn};
use crate::{
    registry_impl::{core_err, SqliteControlPlane},
    Store, StoreResult,
};
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveValue::Set,
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::{
    jobs::{GitlabJobLookup, GitlabJobsStore, GitlabObservationSource, JobSummary},
    ports::gitlab::{GitlabJobDetail, GitlabRunningJob},
    registry::FleetRuntimeGuard,
};

#[async_trait::async_trait]
impl GitlabJobsStore for SqliteControlPlane {
    async fn gitlab_job_running(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        generation_id: &str,
        runner_id: u64,
        job: &GitlabRunningJob,
        now: i64,
    ) -> shaula_core::error::CoreResult<bool> {
        self.store()
            .observe_gitlab_running(fleet, guard, generation_id, runner_id, job, now)
            .await
            .map_err(core_err)
    }

    async fn gitlab_jobs_pending(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        now: i64,
    ) -> shaula_core::error::CoreResult<Vec<GitlabJobLookup>> {
        self.store()
            .pending_gitlab_reads(fleet, guard, now)
            .await
            .map_err(core_err)
    }

    async fn gitlab_job_read(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        lookup: &GitlabJobLookup,
        read: Option<&GitlabJobDetail>,
        now: i64,
    ) -> shaula_core::error::CoreResult<bool> {
        self.store()
            .apply_gitlab_read(fleet, guard, lookup, read, now)
            .await
            .map_err(core_err)
    }
}

impl Store {
    async fn observe_gitlab_running(
        &self,
        fleet: &str,
        guard: &FleetRuntimeGuard,
        generation_id: &str,
        runner_id: u64,
        job: &GitlabRunningJob,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.begin().await?;
        let Some(scope) = gitlab_scope::current(&tx, fleet, guard).await? else {
            return Ok(false);
        };
        if !claimed(&tx, fleet, guard, generation_id, runner_id, job).await? {
            return Ok(false);
        }
        let reading = gitlab_projection::Reading {
            project_id: job.project_id,
            project_path: &job.project_path,
            job_id: job.id,
            pipeline_id: job.pipeline_id,
            name: &job.name,
            status: &job.status,
            runner_id: Some(runner_id),
            source: GitlabObservationSource::RunnerJobs,
        };
        let previous = find(&tx, &scope.key, &reading.protocol_id()).await?;
        let id = match previous {
            // A newer exact read or a recorded terminal result wins over this tick.
            Some(previous)
                if now < previous.updated_at
                    || previous.gitlab.as_ref().is_some_and(|s| s.result.is_some()) =>
            {
                previous.id
            }
            previous => {
                let context = gitlab_projection::Context {
                    fleet,
                    incarnation: &guard.incarnation,
                    scope: &scope,
                };
                gitlab_projection::apply(&tx, &context, previous, &reading, now).await?
            }
        };
        // Append-only: a later `runner: null` read never reaches this table.
        links::Entity::insert(links::ActiveModel {
            job_record_id: Set(id),
            generation_id: Set(generation_id.into()),
            runner_id: Set(to_i64(runner_id)?),
            observed_at: Set(now),
        })
        .on_conflict(
            OnConflict::columns([links::Column::JobRecordId, links::Column::GenerationId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
}

/// Only the Generation's persisted claim (spec 0043 §6) is Verified evidence:
/// same Fleet incarnation, same registration, same `project_id/job_id`.
async fn claimed(
    tx: &DatabaseTransaction,
    fleet: &str,
    guard: &FleetRuntimeGuard,
    generation_id: &str,
    runner_id: u64,
    job: &GitlabRunningJob,
) -> StoreResult<bool> {
    let found = associations::Entity::find()
        .select_only()
        .expr(Expr::val(1))
        .inner_join_on(
            generations::Entity,
            same(generations::Column::Id, associations::Column::GenerationId),
        )
        .inner_join_on(
            fleet_revisions::Entity,
            same(
                fleet_revisions::Column::FleetKey,
                generations::Column::FleetKey,
            )
            .and(same(
                fleet_revisions::Column::Revision,
                generations::Column::FleetRevision,
            )),
        )
        .inner_join_on(
            identities::Entity,
            same(identities::Column::GenerationId, generations::Column::Id),
        )
        .filter(associations::Column::GenerationId.eq(generation_id))
        .filter(generations::Column::FleetKey.eq(fleet))
        .filter(fleet_revisions::Column::Incarnation.eq(guard.incarnation.as_str()))
        .filter(identities::Column::RunnerId.eq(to_i64(runner_id)?))
        .filter(associations::Column::JobId.eq(to_i64(job.id)?))
        .filter(associations::Column::ProjectId.eq(to_i64(job.project_id)?))
        .into_tuple::<i64>()
        .all(tx)
        .await?;
    Ok(!found.is_empty())
}

pub(super) async fn find(
    tx: &DatabaseTransaction,
    scope: &str,
    protocol_id: &str,
) -> StoreResult<Option<JobSummary>> {
    jobs_table::Entity::find()
        .select_only()
        .column(jobs_table::Column::SummaryJson)
        .filter(jobs_table::Column::ScopeKey.eq(scope))
        .filter(jobs_table::Column::ProtocolJobId.eq(protocol_id))
        .into_tuple::<String>()
        .one(tx)
        .await?
        .map(|json| decode(&json))
        .transpose()
}

pub(super) fn to_i64(id: u64) -> StoreResult<i64> {
    i64::try_from(id)
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| crate::StoreError::Corrupt("invalid GitLab identity".into()))
}
