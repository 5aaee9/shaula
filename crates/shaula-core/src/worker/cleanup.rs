use super::CompletionReceipt;
use crate::state_backend::StateResult;
use std::path::PathBuf;

pub struct WorkspaceCleanup {
    pub receipt: CompletionReceipt,
    pub workspace: PathBuf,
    pub artifact: PathBuf,
    pub artifact_digest: String,
    pub retained_input_digest: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CleanupOutcome {
    Reaped,
    Retained,
}

/// Called only after durable terminal receipt AND verified executor fence.
/// Unknown/emergency files are evidence, never ordinary cleanup candidates.
#[async_trait::async_trait]
pub trait WorkspaceReaper: Send + Sync {
    async fn reap(&self, request: WorkspaceCleanup) -> StateResult<CleanupOutcome>;
}
