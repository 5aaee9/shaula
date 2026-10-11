//! Auth Profiles, their revisions, bindings and credential material.

use async_trait::async_trait;

use super::{AuthDependentTarget, AuthRevisionRow, MutationFacts, ProfileHead};
use crate::error::CoreResult;
use crate::registry::auth_port::{AuthPromotion, AuthPromotionOutcome};
use crate::registry::MutationError;

#[async_trait]
pub trait AuthProfileStore: Send + Sync {
    async fn auth_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>>;
    async fn auth_profile_keys(&self) -> CoreResult<Vec<String>>;
    async fn auth_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<AuthRevisionRow>>;
    async fn auth_revision_active(&self, key: &str) -> CoreResult<Option<AuthRevisionRow>>;
    async fn auth_bindings_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Vec<crate::auth_context::AccountBinding>>;
    /// Live Fleet Targets currently desiring this auth profile (active,
    /// blocked and decommissioning phases; tombstones excluded).
    async fn auth_live_dependents(&self, key: &str) -> CoreResult<Vec<AuthDependentTarget>>;
    /// v2-aware promotion: the bindings + snapshot commit ATOMICALLY with
    /// the head advance, after the coverage and fingerprint gates (spec
    /// 0011 §4.1/§5.1).
    async fn auth_apply_validation_v2(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
        promotion: Option<AuthPromotion>,
    ) -> CoreResult<AuthPromotionOutcome>;
    /// Protected-memory handoff of credential bytes for the EXACT
    /// revision; used for idempotency comparison, never persisted or
    /// logged. Returns None when the revision does not exist.
    async fn auth_credential_bytes(&self, key: &str, revision: i64) -> CoreResult<Option<Vec<u8>>>;
    /// Commits one Auth Candidate revision with credential bytes.
    async fn commit_auth_revision(
        &self,
        facts: MutationFacts,
        credential: AuthRevisionRow,
        secret_bytes: &[u8],
    ) -> CoreResult<Result<(), MutationError>>;
    /// Serializes replay, desired-head and Active-base checks with credential
    /// inheritance and Candidate/change/audit/outbox publication. Replay never
    /// needs the historical credential to remain available.
    async fn commit_auth_policy_update(
        &self,
        facts: MutationFacts,
        base_revision: i64,
        policy_json: String,
    ) -> CoreResult<Result<crate::registry::MutationAccepted, MutationError>>;
}
