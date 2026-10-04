//! Recovery facts and persistence port; no database or executor implementation.
use super::{cleanup::WorkspaceCleanup, CompletionReceipt, FenceOutcome, ProcessIdentity};
use crate::state_backend::{StateClaim, StateError, StateResult};
use std::path::Path;
use uuid::Uuid;

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
    pub fn claim(&self) -> StateResult<StateClaim> {
        Ok(StateClaim {
            generation_id: Uuid::parse_str(&self.generation_id).map_err(|_| StateError::Invalid)?,
            worker_epoch: self.worker_epoch,
            worker_attempt: Uuid::parse_str(&self.worker_attempt)
                .map_err(|_| StateError::Invalid)?,
        })
    }
}

pub struct CompletedWorkspace {
    pub cleanup: WorkspaceCleanup,
    pub process: Option<ProcessIdentity>,
}

#[async_trait::async_trait]
pub trait RecoveryJournal: Send + Sync {
    async fn recovery_records(&self) -> StateResult<Vec<RecoveryRecord>>;
    async fn recover_worker(
        &self,
        record: &RecoveryRecord,
        outcome: FenceOutcome,
        preserve_evidence: bool,
    ) -> StateResult<()>;
    async fn next_completed_workspace(
        &self,
        after: &str,
        artifact_root: &Path,
    ) -> StateResult<Option<CompletedWorkspace>>;
    async fn workspace_reaped(&self, receipt: &CompletionReceipt) -> StateResult<()>;
}
