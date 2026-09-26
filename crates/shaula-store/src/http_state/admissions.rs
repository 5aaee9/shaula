//! One-time credential handoff for committed admissions in this daemon epoch.
use std::{collections::HashMap, sync::Mutex};

use sea_orm::{ConnectionTrait, DatabaseTransaction};
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::WorkerAdmission,
};
use uuid::Uuid;

use super::{sql, unavailable, SqliteStateBackend};

pub struct WorkerAdmissions {
    pending: Mutex<HashMap<Uuid, WorkerAdmission>>,
    create_limit: usize,
}

impl Default for WorkerAdmissions {
    fn default() -> Self {
        Self {
            pending: Mutex::default(),
            create_limit: 56,
        }
    }
}

impl WorkerAdmissions {
    pub(crate) fn discard(&self, id: &str) {
        if let (Ok(id), Ok(mut pending)) = (Uuid::parse_str(id), self.pending.lock()) {
            pending.remove(&id);
        }
    }
    pub fn new(max_workers: usize, recovery_reserve: usize) -> StateResult<Self> {
        if recovery_reserve == 0 || recovery_reserve >= max_workers || max_workers > 1024 {
            return Err(StateError::Invalid);
        }
        Ok(Self {
            pending: Mutex::default(),
            create_limit: max_workers - recovery_reserve,
        })
    }
    /// Removed exactly once. Missing credentials never authorize a relaunch;
    /// the durable pending launch must instead be classified by recovery.
    pub fn take(&self, id: Uuid) -> StateResult<WorkerAdmission> {
        self.pending
            .lock()
            .map_err(|_| StateError::Unavailable)?
            .remove(&id)
            .ok_or(StateError::Conflict)
    }

    pub(crate) async fn admit_on(
        &self,
        tx: &DatabaseTransaction,
        id: &str,
    ) -> StateResult<WorkerAdmission> {
        let mode = tx.query_one(sql(
            "SELECT activated FROM lifecycle_deployment WHERE singleton = 1 AND format_version = 1",
            vec![],
        )).await.map_err(unavailable)?.ok_or(StateError::Unavailable)?;
        if !mode.try_get::<bool>("", "activated").map_err(unavailable)? {
            return Err(StateError::Conflict);
        }
        let count = tx.query_one(sql("SELECT COUNT(*) AS count FROM lifecycle_workers WHERE phase NOT IN ('completed', 'quarantined')", vec![])).await.map_err(unavailable)?.ok_or(StateError::Unavailable)?.try_get::<i64>("", "count").map_err(unavailable)?;
        if count >= self.create_limit as i64 {
            return Err(StateError::Unavailable);
        }
        let admission = WorkerAdmission::new(Uuid::parse_str(id).map_err(|_| StateError::Invalid)?);
        SqliteStateBackend::admit_worker_on(tx, &admission).await?;
        Ok(admission)
    }

    pub(crate) fn publish(&self, admission: WorkerAdmission) -> StateResult<()> {
        let mut pending = self.pending.lock().map_err(|_| StateError::Unavailable)?;
        if pending.contains_key(&admission.claim.generation_id) {
            return Err(StateError::Conflict);
        }
        pending.insert(admission.claim.generation_id, admission);
        Ok(())
    }
}
