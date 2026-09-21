//! Provider-independent hard lifetime, separate from idle retirement and IaC timeouts.

use std::time::Duration;

use async_trait::async_trait;

use crate::error::CoreResult;

pub const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(2 * 60 * 60);

/// Durable checkpoints: neither readiness updates nor process restarts reset the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunnerLifetime {
    pub provisioned_at: Option<i64>,
    pub expiry_requested_at: Option<i64>,
    pub resources_destroyed_at: Option<i64>,
}

impl RunnerLifetime {
    /// A committed expiry remains in force even if the configured limit later increases.
    pub fn is_due(self, now: i64, max_lifetime: Duration) -> bool {
        self.expiry_requested_at.is_some()
            || self.provisioned_at.is_some_and(|created| {
                u128::try_from(now.saturating_sub(created))
                    .is_ok_and(|elapsed| elapsed >= max_lifetime.as_millis())
            })
    }
}

#[async_trait]
pub trait RunnerLifetimeStore: Send + Sync {
    async fn generation_lifetime(&self, id: &str) -> CoreResult<RunnerLifetime>;
    /// Commit expiry intent before any destructive effect. Never revives a terminal
    /// or quarantined generation, and requires a successful Create checkpoint.
    async fn generation_request_expiry(&self, id: &str, now: i64) -> CoreResult<bool>;
    /// Record successful resource destruction, independently of remote deregistration.
    async fn generation_resources_destroyed(&self, id: &str, now: i64) -> CoreResult<()>;
}
