//! Test-only transport outage at the real Terraform final POST boundary.
use shaula_core::state_backend::*;
use shaula_store::http_state::SqliteStateBackend;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
