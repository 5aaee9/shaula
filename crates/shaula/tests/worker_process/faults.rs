//! Test-only transport outage at the real Terraform final POST boundary.
use shaula_core::state_backend::*;
use shaula_core::worker::{wire::*, ControlCapability};
use shaula_store::http_state::SqliteStateBackend;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

pub struct ControlFaults {
    pub workers: Arc<shaula_daemon::workers::Workers>,
    pub lost_receipts: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkerControl for ControlFaults {
    async fn authenticate(&self, generation: Uuid, token: &ControlCapability) -> StateResult<()> {
        self.workers.authenticate(generation, token).await
    }
    async fn material(
        &self,
        generation: Uuid,
        token: &ControlCapability,
        id: Uuid,
    ) -> StateResult<Vec<u8>> {
        self.workers.material(generation, token, id).await
    }
    async fn logs(
        &self,
        generation: Uuid,
        token: &ControlCapability,
        request: LogRequest,
    ) -> StateResult<LogResponse> {
        self.workers.logs(generation, token, request).await
    }
    async fn call(
        &self,
        token: &ControlCapability,
        request: ControlRequest,
    ) -> StateResult<ControlResponse> {
        let response = self.workers.call(token, request).await?;
        if matches!(response, ControlResponse::Desired(Directive::Complete(_)))
            && self
                .lost_receipts
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
        {
            // Durable completion has committed but the client sees a transport
            // failure instead of its acknowledgement. Retry must not reopen it.
            return Err(StateError::Unavailable);
        }
        Ok(response)
    }
}

pub struct StateFaults {
    pub backend: SqliteStateBackend,
    pub reject_empty_writes: AtomicBool,
    pub rejected: AtomicUsize,
}

#[async_trait::async_trait]
impl StateBackend for StateFaults {
    async fn authenticate(&self, access: &StateAccess) -> StateResult<()> {
        self.backend.authenticate(access).await
    }
    async fn read(&self, access: &StateAccess) -> StateResult<Option<StateSnapshot>> {
        self.backend.read(access).await
    }
    async fn lock(&self, access: &StateAccess, info: LockInfo) -> StateResult<()> {
        self.backend.lock(access, info).await
    }
    async fn unlock(&self, access: &StateAccess, id: &LockId) -> StateResult<()> {
        self.backend.unlock(access, id).await
    }
    async fn write(
        &self,
        access: &StateAccess,
        id: &LockId,
        document: StateDocument,
    ) -> StateResult<i64> {
        if document.managed_empty() && self.reject_empty_writes.load(Ordering::Acquire) {
            self.rejected.fetch_add(1, Ordering::AcqRel);
            return Err(StateError::Unavailable);
        }
        self.backend.write(access, id, document).await
    }
}
pub struct ReaperFaults {
    pub failures: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub inner: shaula_template::cleanup::ReceiptWorkspaceReaper,
}

#[async_trait::async_trait]
impl shaula_core::worker::cleanup::WorkspaceReaper for ReaperFaults {
    async fn reap(
        &self,
        request: shaula_core::worker::cleanup::WorkspaceCleanup,
    ) -> shaula_core::state_backend::StateResult<shaula_core::worker::cleanup::CleanupOutcome> {
        use std::sync::atomic::Ordering;
        if self
            .failures
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(shaula_core::state_backend::StateError::Unavailable);
        }
        self.inner.reap(request).await
    }
}
