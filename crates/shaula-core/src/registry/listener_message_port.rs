//! Durable message ingestion and outbound acquisition decisions.

use async_trait::async_trait;

use crate::auth_context::ResolvedAuthContext;
use crate::error::CoreResult;
use crate::ports::PollMessage;

use super::FleetRuntimeGuard;

/// The complete authority captured by one installed listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEffectContext {
    pub guard: FleetRuntimeGuard,
    pub auth_context: ResolvedAuthContext,
    pub epoch: i64,
}

/// A committed message whose remaining network work can be recovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestedMessage {
    pub message_id: i64,
    pub acked: bool,
    pub pending_request_ids: Vec<i64>,
}

#[async_trait]
pub trait ListenerMessageStore: Send + Sync {
    /// Rechecks current head, authorization and epoch immediately before ACK.
    async fn listener_current(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
    ) -> CoreResult<bool>;

    /// Commits statistics, observations, acquisition intents and wake atomically.
    /// A duplicate never rewrites a newer snapshot. None means stale authority.
    async fn listener_ingest(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message: &PollMessage,
        now: i64,
    ) -> CoreResult<Option<IngestedMessage>>;

    /// Recover unacknowledged messages and acknowledged, unstarted acquisitions.
    async fn listener_pending(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
    ) -> CoreResult<Vec<IngestedMessage>>;

    /// Records successful ACK and advances only the acknowledged checkpoint.
    async fn listener_acknowledge(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        now: i64,
    ) -> CoreResult<bool>;

    /// Atomically selects Pending requests and records AcquireStarting.
    /// None is stale; an empty vector has no new outbound work.
    async fn listener_acquire_start(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        now: i64,
    ) -> CoreResult<Option<Vec<i64>>>;

    /// None is an uncertain outcome and is never blindly retried. A definite
    /// result records accepted/rejected IDs; an empty result rejects all.
    /// Even stale completions retain their outcome, but cannot update demand.
    async fn listener_acquire_complete(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        accepted: Option<&[i64]>,
        now: i64,
    ) -> CoreResult<bool>;
}
