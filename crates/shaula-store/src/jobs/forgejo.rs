//! Snapshot projection for Forgejo, isolated from GitHub listener observations.
use super::{decode, encode, execute, forgejo_projection, rows};
use crate::{
    registry_impl::{core_err, SqliteControlPlane},
    Store, StoreError, StoreResult,
};
use sha2::{Digest, Sha256};
use shaula_core::{
    fleet::{FleetProviderKind, FleetSpec},
    jobs::{ForgejoJobsStore, JobSummary, ObservedStatus},
    ports::forgejo::ForgejoJob,
    registry::FleetRuntimeGuard,
};
use std::collections::BTreeSet;

#[async_trait::async_trait]
impl ForgejoJobsStore for SqliteControlPlane {
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
        let found = rows(
            &tx,
            "SELECT r.spec_json FROM fleets f JOIN fleet_revisions r
            ON r.fleet_key=f.key AND r.incarnation=f.incarnation AND r.revision=f.desired_revision
            WHERE f.key=? AND f.incarnation=? AND f.desired_revision=? AND f.mutation_fence=?
            AND f.deletion_marker=0 AND f.tombstone=0",
            vec![
                fleet.into(),
                guard.incarnation.clone().into(),
                guard.desired_revision.into(),
                guard.mutation_fence.into(),
            ],
        )
        .await?;
        let Some(row) = found.first() else {
            return Ok(false);
        };
        let spec: FleetSpec = decode(&row.try_get::<String>("", "spec_json")?)?;
        if spec.kind != FleetProviderKind::Forgejo {
            return Ok(false);
        }
        let section = spec
            .forgejo
            .ok_or_else(|| StoreError::Corrupt("Forgejo target missing".into()))?;
        let target = section.target();
        let scope = hex::encode(Sha256::digest(
            encode(&(
                fleet,
                &guard.incarnation,
                &target,
                &section.auth_profile_ref,
            ))?
            .as_bytes(),
        ));
        let polls = rows(
            &tx,
            "SELECT attempted_at FROM forgejo_job_polls WHERE scope_key=?",
            vec![scope.clone().into()],
        )
        .await?;
        if polls
            .first()
            .map(|row| row.try_get::<i64>("", "attempted_at"))
            .transpose()?
            .is_some_and(|previous| now < previous)
        {
            return Ok(false);
        }
        let Some(jobs) = jobs else {
            execute(&tx, "INSERT INTO forgejo_job_polls(scope_key,observed_at,attempted_at,failed) VALUES(?,0,?,1)
                ON CONFLICT(scope_key) DO UPDATE SET attempted_at=excluded.attempted_at,failed=1", vec![scope.into(), now.into()]).await?;
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
        for row in rows(
            &tx,
            "SELECT id,summary_json FROM forgejo_workflow_jobs WHERE scope_key=? AND in_snapshot=1",
            vec![scope.clone().into()],
        )
        .await?
        {
            let id: String = row.try_get("", "id")?;
            if present.contains(&id) {
                continue;
            }
            let mut summary: JobSummary = decode(&row.try_get::<String>("", "summary_json")?)?;
            let state = summary
                .forgejo
                .as_mut()
                .ok_or_else(|| StoreError::Corrupt("Forgejo job metadata missing".into()))?;
            state.in_snapshot = false;
            let task_id = state.task_id.clone();
            summary.observed_status = ObservedStatus::Unknown;
            summary.updated_at = now;
            // Last listed time stays attributed to the actual job. Absence is
            // a separate retained observation, never a synthetic success/result.
            execute(&tx, "UPDATE forgejo_workflow_jobs SET summary_json=?,status='unknown',in_snapshot=0,updated_at=? WHERE id=?",
                vec![encode(&summary)?.into(), now.into(), id.clone().into()]).await?;
            forgejo_projection::event(&tx, &id, None, &task_id, now).await?;
        }
        execute(&tx, "INSERT INTO forgejo_job_polls(scope_key,observed_at,attempted_at,failed) VALUES(?,?,?,0)
            ON CONFLICT(scope_key) DO UPDATE SET observed_at=excluded.observed_at,attempted_at=excluded.attempted_at,failed=0",
            vec![scope.into(), now.into(), now.into()]).await?;
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
