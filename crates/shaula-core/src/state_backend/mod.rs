//! Private Terraform HTTP backend contract. No management credentials or
//! client-supplied lock metadata confer Generation ownership.

mod access;
mod document;
mod lock;

#[cfg(test)]
mod tests;

pub use access::{StateAccess, StateCapability, StateClaim};
pub use document::StateDocument;
pub use lock::{LockId, LockInfo};

/// Hard bounds apply at both the HTTP and persistence boundaries.
pub const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_LOCK_BYTES: usize = 16 * 1024;

/// Safe to log: errors never embed state, lock IDs, credentials or SQL.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state access denied")]
    Unauthorized,
    #[error("invalid backend request")]
    Invalid,
    #[error("backend request too large")]
    TooLarge,
    #[error("state ownership or version conflict")]
    Conflict,
    #[error("state is locked")]
    Locked(Box<LockInfo>),
    #[error("state is sealed")]
    Sealed,
    #[error("authoritative state unavailable")]
    Unavailable,
}

pub type StateResult<T> = Result<T, StateError>;

/// `None` is exclusively an admitted, never-initialized backend. An unknown
/// Generation, lost row or corrupt document must never look like fresh state.
#[derive(Debug)]
pub struct StateSnapshot {
    pub revision: i64,
    pub document: StateDocument,
}

/// Worker-facing backend only. Admission, revocation and terminal sealing
/// belong to the daemon, and are deliberately absent from this port.
#[async_trait::async_trait]
pub trait StateBackend: Send + Sync {
    /// Authentication before reading the HTTP body. Every operation MUST
    /// revalidate ownership in its own transaction (revocation can race I/O).
    async fn authenticate(&self, access: &StateAccess) -> StateResult<()>;
    async fn read(&self, access: &StateAccess) -> StateResult<Option<StateSnapshot>>;
    async fn lock(&self, access: &StateAccess, info: LockInfo) -> StateResult<()>;
    async fn unlock(&self, access: &StateAccess, id: &LockId) -> StateResult<()>;
    async fn write(
        &self,
        access: &StateAccess,
        id: &LockId,
        document: StateDocument,
    ) -> StateResult<i64>;
}
