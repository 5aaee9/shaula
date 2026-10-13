//! Claim-time projection for GitLab, isolated from GitHub and Forgejo jobs.
use super::{decode, execute, gitlab_projection, gitlab_scope, rows};
use crate::{
    registry_impl::{core_err, SqliteControlPlane},
    Store, StoreResult,
};
use sea_orm::DatabaseTransaction;
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
        execute(
            &tx,
            "INSERT INTO gitlab_job_generations(job_record_id,generation_id,runner_id,observed_at)
            VALUES(?,?,?,?) ON CONFLICT(job_record_id,generation_id) DO NOTHING",
            vec![
                id.into(),
                generation_id.into(),
                to_i64(runner_id)?.into(),
                now.into(),
            ],
        )
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
    let found = rows(
        tx,
        "SELECT 1 FROM gitlab_job_associations a JOIN runner_generations g ON g.id=a.generation_id
         JOIN fleet_revisions r ON r.fleet_key=g.fleet_key AND r.revision=g.fleet_revision
         JOIN gitlab_runner_identities i ON i.generation_id=g.id
         WHERE a.generation_id=? AND g.fleet_key=? AND r.incarnation=? AND i.runner_id=?
         AND a.job_id=? AND a.project_id=?",
        vec![
            generation_id.into(),
            fleet.into(),
            guard.incarnation.clone().into(),
            to_i64(runner_id)?.into(),
            to_i64(job.id)?.into(),
            to_i64(job.project_id)?.into(),
        ],
    )
    .await?;
    Ok(!found.is_empty())
}

pub(super) async fn find(
    tx: &DatabaseTransaction,
    scope: &str,
    protocol_id: &str,
) -> StoreResult<Option<JobSummary>> {
    rows(
        tx,
        "SELECT summary_json FROM gitlab_workflow_jobs WHERE scope_key=? AND protocol_job_id=?",
        vec![scope.into(), protocol_id.into()],
    )
    .await?
    .first()
    .map(|row| decode(&row.try_get::<String>("", "summary_json")?))
    .transpose()
}

pub(super) fn to_i64(id: u64) -> StoreResult<i64> {
    i64::try_from(id)
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| crate::StoreError::Corrupt("invalid GitLab identity".into()))
}
