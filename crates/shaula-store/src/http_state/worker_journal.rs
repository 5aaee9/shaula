use super::{sql, unavailable, worker::WorkerRow, SqliteStateBackend, WorkerAdmissions};
use sea_orm::ConnectionTrait;
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::{
        journal::WorkerJournal, CompletionReceipt, ControlAccess, ProcessIdentity, WorkerAdmission,
    },
};
use std::sync::Arc;
use uuid::Uuid;

pub struct SqliteWorkerJournal {
    pub(super) backend: SqliteStateBackend,
    pub(super) admissions: Arc<WorkerAdmissions>,
}

impl SqliteWorkerJournal {
    pub fn new(backend: SqliteStateBackend, admissions: Arc<WorkerAdmissions>) -> Self {
        Self {
            backend,
            admissions,
        }
    }
}

#[async_trait::async_trait]
impl WorkerJournal for SqliteWorkerJournal {
    async fn workspace_reaped(&self, receipt: &CompletionReceipt) -> StateResult<()> {
        self.backend.workspace_reaped(receipt).await
    }
    async fn replace_fenced(&self, access: &ControlAccess) -> StateResult<()> {
        self.backend.authenticate_worker(access).await?;
        let record = self
            .backend
            .recovery_records()
            .await?
            .into_iter()
            .find(|record| record.generation_id == access.claim.generation_id.to_string())
            .ok_or(StateError::Conflict)?;
        if record.phase != "fenced" {
            return Err(StateError::Conflict);
        }
        self.backend
            .recover_worker(
                &record,
                shaula_core::worker::FenceOutcome::Fenced,
                &self.admissions,
                false,
            )
            .await
    }
    async fn retain_input(&self, access: &ControlAccess, input: Vec<u8>) -> StateResult<()> {
        if input.len() > shaula_core::state_backend::MAX_STATE_BYTES {
            return Err(StateError::TooLarge);
        }
        let id = access.claim.generation_id.to_string();
        let tx = self.backend.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        let state = super::row::Row::load(&tx, &id).await?;
        if worker.phase != "running" || worker.cleanup_only || state.create_started {
            return Err(StateError::Conflict);
        }
        let row = tx
            .query_one(sql(
                "SELECT protected_input FROM lifecycle_workers WHERE generation_id = ?",
                vec![id.clone().into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?;
        if let Some(saved) = row
            .try_get::<Option<Vec<u8>>>("", "protected_input")
            .map_err(unavailable)?
        {
            return if saved == input {
                Ok(())
            } else {
                Err(StateError::Conflict)
            };
        }
        tx.execute(sql(
            "UPDATE lifecycle_workers SET protected_input = ? WHERE generation_id = ?",
            vec![input.into(), id.into()],
        ))
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
    async fn protected_input(&self, access: &ControlAccess) -> StateResult<Option<Vec<u8>>> {
        WorkerRow::load(self.backend.store.connection(), access).await?;
        self.backend
            .store
            .connection()
            .query_one(sql(
                "SELECT protected_input FROM lifecycle_workers WHERE generation_id = ?",
                vec![access.claim.generation_id.to_string().into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?
            .try_get::<Option<Vec<u8>>>("", "protected_input")
            .map_err(unavailable)
    }
    async fn verify_cleanup(&self, access: &ControlAccess) -> StateResult<()> {
        let id = access.claim.generation_id.to_string();
        let tx = self.backend.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        if worker.phase != "running" || worker.handover.is_some() || worker.active_command.is_some()
        {
            return Err(StateError::Conflict);
        }
        let state = super::row::Row::load(&tx, &id).await?;
        let snapshot = state.snapshot(&tx).await?.ok_or(StateError::Unavailable)?;
        if state.lock(&tx).await?.is_some() || !snapshot.document.managed_empty() {
            return Err(StateError::Conflict);
        }
        if state.create_started {
            if !super::create_proof::verified(&tx, &id, &snapshot.document).await? {
                return Err(StateError::Conflict);
            }
            let seen = tx
                .query_one(sql(
                    "SELECT resource_state_seen FROM lifecycle_workers WHERE generation_id = ?",
                    vec![id.clone().into()],
                ))
                .await
                .map_err(unavailable)?
                .ok_or(StateError::Unavailable)?
                .try_get::<bool>("", "resource_state_seen")
                .map_err(unavailable)?;
            if !seen {
                return Err(StateError::Conflict);
            }
        }
        tx.execute(sql(
            "UPDATE lifecycle_workers SET cleanup_revision = ? WHERE generation_id = ?",
            vec![state.revision.into(), id.into()],
        ))
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }

    async fn admission(&self, generation: Uuid) -> StateResult<WorkerAdmission> {
        self.admissions.take(generation)
    }
    async fn authenticate(&self, access: &ControlAccess) -> StateResult<()> {
        self.backend.authenticate_worker(access).await
    }
    async fn launch_pending(&self, access: &ControlAccess) -> StateResult<()> {
        self.backend.worker_launch_pending(access).await
    }
    async fn register(
        &self,
        access: &ControlAccess,
        identity: &ProcessIdentity,
    ) -> StateResult<()> {
        self.backend.worker_register(access, identity).await
    }
    async fn spawn_ack(&self, access: &ControlAccess, attempt: &str) -> StateResult<()> {
        let id = access.claim.generation_id.to_string();
        let tx = self.backend.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        if worker.phase != "running"
            || worker.active_command.as_deref() != Some(attempt)
            || worker.handover.as_deref().is_some_and(|id| id != attempt)
        {
            return Err(StateError::Conflict);
        }
        tx.execute(sql(
            "UPDATE lifecycle_workers SET handover = NULL WHERE generation_id = ?",
            vec![id.into()],
        ))
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
    async fn command_ended(&self, access: &ControlAccess, attempt: &str) -> StateResult<()> {
        let id = access.claim.generation_id.to_string();
        let tx = self.backend.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        if worker.phase != "running"
            || worker.handover.is_some()
            || worker.active_command.as_deref() != Some(attempt)
        {
            return Err(StateError::Conflict);
        }
        tx.execute(sql(
            "UPDATE lifecycle_workers SET active_command = NULL WHERE generation_id = ?",
            vec![id.into()],
        ))
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
    async fn fenced(&self, access: &ControlAccess, identity: &ProcessIdentity) -> StateResult<()> {
        let id = access.claim.generation_id.to_string();
        let tx = self.backend.writer(&id).await?;
        let worker = WorkerRow::load(&tx, access).await?;
        let saved: ProcessIdentity = serde_json::from_str(
            worker
                .process_identity
                .as_deref()
                .ok_or(StateError::Conflict)?,
        )
        .map_err(|_| StateError::Unavailable)?;
        if saved != *identity || worker.phase == "completed" {
            return Err(StateError::Conflict);
        }
        tx.execute(sql("UPDATE lifecycle_workers SET phase = 'fenced', handover = NULL, active_command = NULL WHERE generation_id = ?", vec![id.into()])).await.map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }
    async fn receipt(&self, access: &ControlAccess) -> StateResult<Option<CompletionReceipt>> {
        let worker = WorkerRow::load(self.backend.store.connection(), access).await?;
        worker
            .completion_receipt
            .map(|json| serde_json::from_str(&json).map_err(|_| StateError::Unavailable))
            .transpose()
    }
}
