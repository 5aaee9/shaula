//! Fleet auth handoff tuples and Resolved Auth Context persistence.

use async_trait::async_trait;

use super::{AuthHandoffRow, FleetAuthContextRow};
use crate::error::CoreResult;
use crate::registry::auth_port::FleetContextAck;

#[async_trait]
pub trait HandoffStore: Send + Sync {
    async fn handoff_get(&self, fleet_key: &str) -> CoreResult<Option<AuthHandoffRow>>;
    /// Durably advances the observed auth tuple after successful
    /// classification. With `context_json` (a v2 exact Resolved Auth
    /// Context), the observed context advances in the SAME transaction
    /// after the durable-pin check; a pin conflict writes NOTHING and
    /// reports [`FleetContextAck::IdentityDrift`].
    async fn handoff_acknowledge(
        &self,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
        context_json: Option<&str>,
        expectation: &crate::registry::AuthHandoffExpectation,
    ) -> CoreResult<FleetContextAck>;
    /// Records a failed proof only while its captured authority is current.
    /// False means stale work was discarded without changing either rollout.
    async fn handoff_mark_blocked(
        &self,
        fleet_key: &str,
        authority: &(String, i64),
        expectation: &crate::registry::AuthHandoffExpectation,
        reason: &str,
        retry_at: i64,
    ) -> CoreResult<bool>;
    /// The persisted desired/observed Resolved Auth Context of a fleet.
    async fn fleet_auth_context_get(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<FleetAuthContextRow>>;
    /// Marks the context resolution blocked with a sanitized reason and
    /// bounded retry deadline.
    async fn fleet_auth_context_block(
        &self,
        fleet_key: &str,
        reason: &str,
        retry_at: i64,
        now: i64,
    ) -> CoreResult<()>;
}
