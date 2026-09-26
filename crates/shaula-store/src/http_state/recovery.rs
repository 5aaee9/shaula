//! Restart classification under the daemon ownership lock, before acquisition.
use super::{row::Row, sql, unavailable, worker::WorkerRow, SqliteStateBackend, WorkerAdmissions};
use sea_orm::{ConnectionTrait, FromQueryResult};
use shaula_core::{
    state_backend::{StateClaim, StateError, StateResult},
    worker::{FenceOutcome, ProcessIdentity, WorkerAdmission},
};
use uuid::Uuid;

#[derive(FromQueryResult)]
pub struct RecoveryRecord {
    pub generation_id: String,
    pub worker_epoch: i64,
    pub worker_attempt: String,
    pub phase: String,
    pub process_identity: Option<String>,
    pub workspace_path: String,
    pub create_started: bool,
    pub has_input: bool,
}

impl RecoveryRecord {
    pub fn identity(&self) -> StateResult<Option<ProcessIdentity>> {
        self.process_identity
            .as_deref()
            .map(|value| serde_json::from_str(value).map_err(|_| StateError::Unavailable))
            .transpose()
    }
    fn claim(&self) -> StateResult<StateClaim> {
        Ok(StateClaim {
            generation_id: Uuid::parse_str(&self.generation_id).map_err(|_| StateError::Invalid)?,
            worker_epoch: self.worker_epoch,
            worker_attempt: Uuid::parse_str(&self.worker_attempt)
                .map_err(|_| StateError::Invalid)?,
        })
    }
}

impl SqliteStateBackend {
    pub async fn recovery_records(&self) -> StateResult<Vec<RecoveryRecord>> {
        let rows = self.store.connection().query_all(sql("SELECT w.generation_id, s.worker_epoch, s.worker_attempt, w.phase, w.process_identity, g.workspace_path, s.create_started, w.protected_input IS NOT NULL AS has_input FROM lifecycle_workers w JOIN generation_http_state s ON s.generation_id = w.generation_id JOIN runner_generations g ON g.id = w.generation_id WHERE w.phase != 'completed' AND g.state != 'Destroyed' ORDER BY g.created_at, g.id", vec![])).await.map_err(unavailable)?;
        rows.iter()
            .map(|row| RecoveryRecord::from_query_result(row, "").map_err(unavailable))
            .collect()
    }

    /// `outcome` comes exclusively from the trusted local Executor (or an
    /// admitted record which never entered launch_pending), never worker IPC.
    pub async fn recover_worker(
        &self,
        record: &RecoveryRecord,
        outcome: FenceOutcome,
        admissions: &WorkerAdmissions,
        preserve_evidence: bool,
    ) -> StateResult<()> {
        let tx = self.writer(&record.generation_id).await?;
        let state = Row::load(&tx, &record.generation_id).await?;
        state.check_claim(&record.claim()?)?;
        let worker = WorkerRow::read(&tx, &record.generation_id)
            .await?
            .ok_or(StateError::Conflict)?;
        if worker.phase != record.phase
            || worker.process_identity != record.process_identity
            || state.sealed
        {
            return Err(StateError::Conflict);
        }
        let fenced = outcome == FenceOutcome::Fenced;
        if worker.phase == "quarantined" {
            // A later reboot/verified fence may make explicit operator finalize
            // safe, but never silently resumes a quarantined Generation.
            if fenced {
                tx.execute(sql("UPDATE lifecycle_fences SET confirmed_fenced = 1 WHERE worker_attempt = ? AND generation_id = ? AND worker_epoch = ?", vec![record.worker_attempt.clone().into(), record.generation_id.clone().into(), record.worker_epoch.into()])).await.map_err(unavailable)?;
                tx.execute(sql("UPDATE lifecycle_workers SET handover = NULL, active_command = NULL WHERE generation_id = ?", vec![record.generation_id.clone().into()])).await.map_err(unavailable)?;
            }
            return tx.commit().await.map_err(unavailable);
        }
        let lock = tx
            .query_one(sql(
                "SELECT lock_info FROM generation_http_state WHERE generation_id = ?",
                vec![record.generation_id.clone().into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?
            .try_get::<Option<Vec<u8>>>("", "lock_info")
            .map_err(unavailable)?;
        // Resource cleanup may finish before CI registration removal. Its
        // exact empty-state proof survives a fenced restart while that removal
        // is retried, provided no later command, lock or revision superseded it.
        let cleanup_revision = worker.cleanup_revision.filter(|revision| {
            *revision == state.revision
                && worker.handover.is_none()
                && worker.active_command.is_none()
                && lock.is_none()
        });
        tx.execute(sql("INSERT INTO lifecycle_fences(worker_attempt, generation_id, worker_epoch, process_identity, orphan_lock, outcome) VALUES (?, ?, ?, ?, ?, ?)", vec![record.worker_attempt.clone().into(), record.generation_id.clone().into(), record.worker_epoch.into(), record.process_identity.clone().into(), lock.into(), (if fenced { "fenced" } else { "unknown" }).into()])).await.map_err(unavailable)?;
        if !fenced
            || preserve_evidence
            || (record.create_started && !record.has_input)
            || !state
                .snapshot(&tx)
                .await
                .is_ok_and(|snapshot| snapshot.is_some())
        {
            tx.execute(sql(
                "UPDATE generation_http_state SET revoked = 1 WHERE generation_id = ?",
                vec![record.generation_id.clone().into()],
            ))
            .await
            .map_err(unavailable)?;
            tx.execute(sql(
                "UPDATE lifecycle_workers SET phase = 'quarantined' WHERE generation_id = ?",
                vec![record.generation_id.clone().into()],
            ))
            .await
            .map_err(unavailable)?;
            tx.execute(sql(
                "UPDATE runner_generations SET state = 'Quarantined' WHERE id = ?",
                vec![record.generation_id.clone().into()],
            ))
            .await
            .map_err(unavailable)?;
            if fenced {
                tx.execute(sql("UPDATE lifecycle_workers SET handover = NULL, active_command = NULL WHERE generation_id = ?", vec![record.generation_id.clone().into()])).await.map_err(unavailable)?;
            }
            return tx.commit().await.map_err(unavailable);
        }
        let mut admission = WorkerAdmission::new(record.claim()?.generation_id);
        admission.claim.worker_epoch = state
            .worker_epoch
            .checked_add(1)
            .ok_or(StateError::Conflict)?;
        admission.cleanup_only = true;
        // Orphan lock removal and credential rotation are a single commit,
        // after the exact old containment was proved empty and audited above.
        tx.execute(sql("UPDATE generation_http_state SET worker_epoch = ?, worker_attempt = ?, capability_hash = ?, revoked = 0, lock_id = NULL, lock_info = NULL WHERE generation_id = ?", vec![admission.claim.worker_epoch.into(), admission.claim.worker_attempt.to_string().into(), admission.state.verifier().into(), record.generation_id.clone().into()])).await.map_err(unavailable)?;
        tx.execute(sql("UPDATE lifecycle_workers SET control_hash = ?, phase = 'admitted', process_identity = NULL, cleanup_only = 1, handover = NULL, active_command = NULL, cleanup_revision = ? WHERE generation_id = ?", vec![admission.control.verifier().into(), cleanup_revision.into(), record.generation_id.clone().into()])).await.map_err(unavailable)?;
        // Recovery never re-enters Create/JIT. Existing CI removal gates still
        // decide when the cleanup-only replacement may actually destroy.
        tx.execute(sql("UPDATE runner_generations SET state = 'CleanupRequired' WHERE id = ? AND state IN ('CreatePending','Creating','WaitingOnline')", vec![record.generation_id.clone().into()])).await.map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        admissions.publish(admission)
    }

    pub(crate) async fn recovered_state_identity(
        &self,
        id: &str,
    ) -> StateResult<Option<(String, u64)>> {
        let Some(worker) = WorkerRow::read(self.store.connection(), id).await? else {
            return Ok(None);
        };
        if !worker.cleanup_only || worker.phase == "quarantined" {
            return Ok(None);
        }
        let row = Row::load(self.store.connection(), id).await?;
        if row.revoked {
            return Err(StateError::Unauthorized);
        }
        let snapshot = row
            .snapshot(self.store.connection())
            .await?
            .ok_or(StateError::Unavailable)?;
        Ok(Some((
            snapshot.document.lineage().into(),
            u64::try_from(snapshot.document.serial()).map_err(|_| StateError::Invalid)?,
        )))
    }
}
