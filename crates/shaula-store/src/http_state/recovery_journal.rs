use super::SqliteWorkerJournal;
use shaula_core::{
    state_backend::StateResult,
    worker::{
        recovery::{CompletedWorkspace, RecoveryJournal, RecoveryRecord},
        CompletionReceipt, FenceOutcome,
    },
};
use std::path::Path;

#[async_trait::async_trait]
impl RecoveryJournal for SqliteWorkerJournal {
    async fn recovery_records(&self) -> StateResult<Vec<RecoveryRecord>> {
        self.backend.recovery_records().await
    }
    async fn recover_worker(
        &self,
        record: &RecoveryRecord,
        outcome: FenceOutcome,
        preserve_evidence: bool,
    ) -> StateResult<()> {
        self.backend
            .recover_worker(record, outcome, &self.admissions, preserve_evidence)
            .await
    }
    async fn next_completed_workspace(
        &self,
        after: &str,
        artifact_root: &Path,
    ) -> StateResult<Option<CompletedWorkspace>> {
        self.backend
            .next_completed_workspace(after, artifact_root)
            .await
    }
    async fn workspace_reaped(&self, receipt: &CompletionReceipt) -> StateResult<()> {
        self.backend.workspace_reaped(receipt).await
    }
}
