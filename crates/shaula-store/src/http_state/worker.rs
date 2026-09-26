//! Worker journal primitives. All checks run after reserving SQLite's writer.
use sea_orm::{ConnectionTrait, DatabaseTransaction, FromQueryResult};
use shaula_core::{
    state_backend::{StateDocument, StateError, StateResult},
    worker::{ControlAccess, ProcessIdentity, WorkerAdmission},
};
use uuid::Uuid;

use super::{exactly_one, row::Row, sql, unavailable, SqliteStateBackend};

#[derive(FromQueryResult)]
pub(super) struct WorkerRow {
    control_hash: Vec<u8>,
    pub phase: String,
    pub process_identity: Option<String>,
    pub cleanup_only: bool,
    pub handover: Option<String>,
    pub active_command: Option<String>,
    pub cleanup_revision: Option<i64>,
    pub completion_request: Option<String>,
    pub completion_receipt: Option<String>,
}

impl WorkerRow {
    pub async fn read(conn: &impl ConnectionTrait, id: &str) -> StateResult<Option<Self>> {
        conn.query_one(sql(
            "SELECT control_hash, phase, process_identity, cleanup_only, handover, active_command,
                cleanup_revision, completion_request, completion_receipt FROM lifecycle_workers WHERE generation_id = ?",
            vec![id.into()],
        )).await.map_err(unavailable)?.map(|row| Self::from_query_result(&row, "").map_err(unavailable)).transpose()
    }

    pub async fn load(tx: &impl ConnectionTrait, access: &ControlAccess) -> StateResult<Self> {
        let state = Row::load(tx, &access.claim.generation_id.to_string()).await?;
        state
            .check_claim(&access.claim)
            .map_err(|_| StateError::Unauthorized)?;
        let worker = Self::read(tx, &access.claim.generation_id.to_string())
            .await?
            .ok_or(StateError::Unauthorized)?;
        if state.revoked || !access.capability.matches(&worker.control_hash) {
            return Err(StateError::Unauthorized);
        }
        Ok(worker)
    }
}

impl SqliteStateBackend {
    /// Composes with the SAME Generation/pool admission transaction. Existing
    /// rows, any CI side effect, or any previous backend are never initialized.
    pub(crate) async fn admit_worker_on(
        tx: &DatabaseTransaction,
        admission: &WorkerAdmission,
    ) -> StateResult<()> {
        if admission.claim.worker_epoch != 1 {
            return Err(StateError::Invalid);
        }
        let id = admission.claim.generation_id.to_string();
        let generation = tx
            .query_one(sql(
                "SELECT state, jit_phase, github_runner_id FROM runner_generations
             WHERE id = ? AND NOT EXISTS (SELECT 1 FROM runner_operations WHERE generation_id = ?)
               AND NOT EXISTS (SELECT 1 FROM forgejo_runner_identities WHERE generation_id = ?)",
                vec![id.clone().into(), id.clone().into(), id.clone().into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Conflict)?;
        if generation
            .try_get::<String>("", "state")
            .map_err(unavailable)?
            != "CreatePending"
            || generation
                .try_get::<Option<String>>("", "jit_phase")
                .map_err(unavailable)?
                .is_some()
            || generation
                .try_get::<Option<i64>>("", "github_runner_id")
                .map_err(unavailable)?
                .is_some()
        {
            return Err(StateError::Conflict);
        }
        // Only a proven fresh admission can manufacture serial zero. init is
        // not required to POST an empty snapshot for the Create gate to work.
        let document = StateDocument::parse(
            serde_json::to_vec(&serde_json::json!({
                "version": 4, "serial": 0, "lineage": Uuid::new_v4().to_string(),
                "outputs": {}, "resources": [],
            }))
            .map_err(|_| StateError::Invalid)?,
        )?;
        tx.execute(sql(
            "INSERT INTO generation_http_state
             (generation_id, worker_epoch, worker_attempt, capability_hash, revision, state_bytes, lineage, serial)
             VALUES (?, ?, ?, ?, 1, ?, ?, 0)",
            vec![id.clone().into(), admission.claim.worker_epoch.into(),
                admission.claim.worker_attempt.to_string().into(), admission.state.verifier().into(),
                document.bytes().to_vec().into(), document.lineage().into()],
        )).await.map_err(unavailable)?;
        tx.execute(sql(
            "INSERT INTO lifecycle_workers(generation_id, control_hash, phase) VALUES (?, ?, 'admitted')",
            vec![id.into(), admission.control.verifier().into()],
        )).await.map_err(unavailable)?;
        Ok(())
    }

    pub async fn authenticate_worker(&self, access: &ControlAccess) -> StateResult<()> {
        WorkerRow::load(self.store.connection(), access).await?;
        Ok(())
    }

    /// Persist before calling exec. A second launcher cannot borrow a pending
    /// launch, even when no process is present in its in-memory child map.
    pub async fn worker_launch_pending(&self, access: &ControlAccess) -> StateResult<()> {
        let id = access.claim.generation_id.to_string();
        let tx = self.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        if worker.phase != "admitted" {
            return Err(StateError::Conflict);
        }
        exactly_one(tx.execute(sql(
            "UPDATE lifecycle_workers SET phase = 'launch_pending' WHERE generation_id = ? AND phase = 'admitted'",
            vec![id.into()],
        )).await.map_err(unavailable)?.rows_affected())?;
        tx.commit().await.map_err(unavailable)
    }

    /// Executor-owned identity, not an arbitrary PID supplied by the Worker.
    /// This method is not exposed directly as a Worker control route.
    pub async fn worker_register(
        &self,
        access: &ControlAccess,
        identity: &ProcessIdentity,
    ) -> StateResult<()> {
        identity.validate()?;
        let identity = serde_json::to_string(identity).map_err(|_| StateError::Invalid)?;
        let id = access.claim.generation_id.to_string();
        let tx = self.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        if worker.phase == "running" && worker.process_identity.as_deref() == Some(&identity) {
            return Ok(());
        }
        if worker.phase != "launch_pending" || worker.process_identity.is_some() {
            return Err(StateError::Conflict);
        }
        tx.execute(sql(
            "UPDATE lifecycle_workers SET phase = 'running', process_identity = ? WHERE generation_id = ?",
            vec![identity.into(), id.into()],
        )).await.map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
}
