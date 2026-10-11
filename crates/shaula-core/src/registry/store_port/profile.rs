//! Cross-kind profile concerns: change reads, retirement and immutable
//! conformance attestations.

use async_trait::async_trait;

use super::MutationFacts;
use crate::error::CoreResult;
use crate::registry::attestation_port::{
    AttestationCommit, AttestationRecord, AttestationReplayRow,
};
use crate::registry::{Actor, ChangeView, MutationError};

#[async_trait]
pub trait ProfileStore: Send + Sync {
    async fn profile_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>>;
    async fn commit_profile_retirement(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Looks up a persisted attestation by its FULL path identity
    /// (Profile, Revision, attestation key). The replay pre-check uses
    /// this to reach the immutable historical record BEFORE the current
    /// engine authority is consulted (R6-07).
    async fn attestation_get(
        &self,
        profile_key: &str,
        revision: i64,
        attestation_key: &str,
    ) -> CoreResult<Option<AttestationReplayRow>>;
    /// Stores immutable conformance evidence in one transaction: replay by
    /// stable identity returns the original record, a conflicting body is
    /// refused. Evidence cannot change activation (spec 0017).
    async fn commit_attestation(
        &self,
        record: AttestationRecord,
        actor: Actor,
        now: i64,
    ) -> CoreResult<Result<AttestationCommit, MutationError>>;
}
