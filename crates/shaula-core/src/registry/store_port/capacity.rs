//! Fleet demand, capacity counters and admission-time generation reads.

use async_trait::async_trait;

use crate::error::CoreResult;
use crate::registry::{Actor, MutationError};

#[async_trait]
pub trait CapacityStore: Send + Sync {
    async fn demand_get(&self, fleet_key: &str) -> CoreResult<Option<i64>>;
    /// Demand plus the time it was last updated, when the store records it.
    async fn demand_with_time(&self, key: &str) -> CoreResult<Option<(i64, Option<i64>)>>;
    async fn capacity_counters(&self, fleet_key: &str) -> CoreResult<(i64, i64)>;
    async fn generations_occupancy(&self, fleet_key: &str) -> CoreResult<i64>;
    /// Single generation read for admission-time checks (e.g. the spec
    /// 0028 finalize gate); supervision-wide iteration stays on
    /// `LifecycleStore`. Named `lookup` so callers importing both ports
    /// see no method-resolution ambiguity.
    async fn generation_lookup(
        &self,
        id: &str,
    ) -> CoreResult<Option<crate::registry::GenerationRecord>>;
    /// Operator finalization of a Quarantined generation (spec 0028):
    /// one transaction CAS-checks `state = Quarantined`, advances to
    /// `Destroyed`, appends the Finalize operation row, the audit fact and
    /// the optional idempotency record. `Err(NotFound)` for a missing row,
    /// `Err(Conflict)` for a non-quarantined state — never a stale silent
    /// success.
    async fn commit_generation_finalize(
        &self,
        generation_id: &str,
        actor: &Actor,
        reason: &str,
        idempotency: Option<crate::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
}
