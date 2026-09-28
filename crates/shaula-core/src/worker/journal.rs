use super::{CompletionReceipt, ControlAccess, ProcessIdentity, WorkerAdmission};
use crate::state_backend::StateResult;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait WorkerJournal: Send + Sync {
    async fn admission(&self, generation: Uuid) -> StateResult<WorkerAdmission>;
    async fn retain_input(&self, access: &ControlAccess, input: Vec<u8>) -> StateResult<()>;
    async fn protected_input(&self, access: &ControlAccess) -> StateResult<Option<Vec<u8>>>;
    async fn replace_fenced(&self, access: &ControlAccess) -> StateResult<()>;
    async fn authenticate(&self, access: &ControlAccess) -> StateResult<()>;
    async fn launch_pending(&self, access: &ControlAccess) -> StateResult<()>;
    async fn register(&self, access: &ControlAccess, identity: &ProcessIdentity)
        -> StateResult<()>;
    async fn spawn_ack(&self, access: &ControlAccess, attempt: &str) -> StateResult<()>;
    async fn command_ended(&self, access: &ControlAccess, attempt: &str) -> StateResult<()>;
    async fn verify_cleanup(&self, access: &ControlAccess) -> StateResult<()>;
    /// Daemon-only, after the Executor proves containment empty. The Worker
    /// never calls this method or manufactures its own fencing receipt.
    async fn fenced(&self, access: &ControlAccess, identity: &ProcessIdentity) -> StateResult<()>;
    async fn receipt(&self, access: &ControlAccess) -> StateResult<Option<CompletionReceipt>>;
    async fn workspace_reaped(&self, receipt: &CompletionReceipt) -> StateResult<()>;
}
