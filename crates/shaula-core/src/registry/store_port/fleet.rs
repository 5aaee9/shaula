//! Fleet desired-state heads, revisions and their atomic commits.

use async_trait::async_trait;

use super::{FleetHead, FleetRevisionRow, MutationFacts};
use crate::error::CoreResult;
use crate::registry::{Actor, ChangeView, MutationError};

#[async_trait]
pub trait FleetStore: Send + Sync {
    async fn fleet_get(&self, key: &str) -> CoreResult<Option<FleetHead>>;
    async fn fleet_list(
        &self,
        actor: &Actor,
        page: crate::registry::KeyPage,
    ) -> CoreResult<Vec<(String, i64, String)>>;
    async fn fleet_count(&self) -> CoreResult<usize>;
    async fn fleet_revision_latest(&self, key: &str) -> CoreResult<Option<FleetRevisionRow>>;
    async fn fleet_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>>;

    /// Commits one effective fleet mutation atomically: revision append,
    /// desired head advance + fence bump, auth handoff desired tuple,
    /// change, audit, outbox and idempotency record.
    // `Err(PreconditionFailed)` maps a lost mutation-fence race (a concurrent
    // DELETE or newer PUT won between admission and commit); the caller
    // maps it onto the HTTP status contract.
    async fn commit_fleet_mutation(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Durably records a NO-OP fleet PUT: an audit entry plus, when the
    /// caller supplied an idempotency key, the stored 200 replay body — so
    /// a retry of the same request replays the original 200 and a key
    /// reuse with a different body conflicts, exactly like a real revision
    /// (spec 0002 §5.2/§5.3). The SAME transaction re-checks the current
    /// head against the no-op's `(incarnation, revision)`; a concurrent
    /// newer PUT or DELETE since the caller's read maps to
    /// `PreconditionFailed` / `Gone` instead of recording a stale no-op.
    async fn commit_fleet_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &Actor,
        idempotency: Option<crate::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Decommission: deletion marker + fence bump + revision + change.
    async fn commit_decommission(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
}
