//! Shared TemplatePool resources (spec 0037).

use async_trait::async_trait;

use super::MutationFacts;
use crate::error::CoreResult;
use crate::registry::{Actor, ChangeView, MutationError};

#[async_trait]
pub trait TemplatePoolStore: Send + Sync {
    /// Desired-state head of one shared TemplatePool.
    async fn template_pool_get(
        &self,
        key: &str,
    ) -> CoreResult<Option<crate::template_pool::TemplatePoolHead>>;
    /// Active (non-tombstoned) pools: (key, desired revision, incarnation).
    async fn template_pool_list(&self) -> CoreResult<Vec<(String, i64, String)>>;
    /// Latest committed pool revision with its resolved member rows.
    async fn template_pool_revision_latest(
        &self,
        key: &str,
    ) -> CoreResult<Option<crate::template_pool::TemplatePoolRevision>>;
    /// Commits a pool mutation atomically: head CAS + revision append +
    /// immutable member rows (carried on `facts.template_pool`) +
    /// change/audit/outbox/idempotency. A lost fence race surfaces as
    /// `PreconditionFailed`.
    async fn commit_template_pool_mutation(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Commits a pool no-op's audit and optional replay atomically after
    /// rechecking its exact live head. No revision/change/outbox is created.
    async fn commit_template_pool_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &Actor,
        idempotency: Option<crate::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Tombstones a pool after the reference check: rejected while any
    /// non-decommissioned fleet revision still references the pool.
    async fn commit_template_pool_delete(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    async fn template_pool_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>>;
}
