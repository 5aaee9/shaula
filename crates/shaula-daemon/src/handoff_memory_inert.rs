//! Inert resource ports of the handoff `MemoryStore` test double: every
//! read is empty and every commit answers a fixed result. These bodies
//! reproduce the answers the double previously inherited from trait
//! defaults, so handoff tests behave identically.

use super::MemoryStore;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    Actor, AdmissionSupportStore, ArtifactStore, CapacityStore, ChangeView, MutationError,
    MutationFacts, ProfileStore, TemplatePoolStore, TemplateStore,
};

#[async_trait::async_trait]
impl CapacityStore for MemoryStore {
    async fn demand_get(&self, _fleet_key: &str) -> CoreResult<Option<i64>> {
        Ok(None)
    }
    async fn demand_with_time(&self, key: &str) -> CoreResult<Option<(i64, Option<i64>)>> {
        Ok(self.demand_get(key).await?.map(|v| (v, None)))
    }
    async fn capacity_counters(&self, _fleet_key: &str) -> CoreResult<(i64, i64)> {
        Ok((0, 0))
    }
    async fn generations_occupancy(&self, _fleet_key: &str) -> CoreResult<i64> {
        Ok(0)
    }
    async fn generation_lookup(
        &self,
        _id: &str,
    ) -> CoreResult<Option<shaula_core::registry::GenerationRecord>> {
        Ok(None)
    }
    async fn commit_generation_finalize(
        &self,
        _generation_id: &str,
        _actor: &Actor,
        _reason: &str,
        _idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        _now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
}

#[async_trait::async_trait]
impl TemplateStore for MemoryStore {
    async fn template_profile_get(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::registry::ProfileHead>> {
        Ok(None)
    }
    async fn template_profile_keys(
        &self,
        _page: shaula_core::registry::KeyPage,
    ) -> CoreResult<Vec<String>> {
        Ok(Vec::new())
    }
    async fn template_references_in_use(&self, _key: &str, _now: i64) -> CoreResult<Option<bool>> {
        Ok(None)
    }
    async fn template_source_get(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::registry::TemplateSource>> {
        Ok(None)
    }
    async fn template_revision_get(
        &self,
        _key: &str,
        _revision: i64,
    ) -> CoreResult<Option<shaula_core::registry::TemplateRevisionRow>> {
        Ok(None)
    }
    async fn template_protected_bindings(
        &self,
        _key: &str,
        _revision: i64,
    ) -> CoreResult<Option<(String, String)>> {
        Ok(None)
    }
    async fn commit_template_revision(
        &self,
        _facts: MutationFacts,
        _extra: (String, String, String, String),
        _source_key: Option<String>,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Ok(()))
    }
    async fn commit_template_noop(
        &self,
        _key: &str,
        _incarnation: &str,
        _revision: i64,
        _actor: &Actor,
        _idempotency: Option<(String, String, String)>,
        _now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Ok(()))
    }
}

#[async_trait::async_trait]
impl TemplatePoolStore for MemoryStore {
    async fn template_pool_get(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::template_pool::TemplatePoolHead>> {
        Ok(None)
    }
    async fn template_pool_list(&self) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(Vec::new())
    }
    async fn template_pool_revision_latest(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::template_pool::TemplatePoolRevision>> {
        Ok(None)
    }
    async fn commit_template_pool_mutation(
        &self,
        _facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
    async fn commit_template_pool_noop(
        &self,
        _key: &str,
        _incarnation: &str,
        _revision: i64,
        _actor: &Actor,
        _idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        _now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
    async fn commit_template_pool_delete(
        &self,
        _facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
    async fn template_pool_change_get(&self, _change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl ArtifactStore for MemoryStore {
    async fn ensure_artifact_cached(&self, _digest: &str) -> CoreResult<()> {
        Ok(())
    }
    async fn artifact_manifest(&self, _digest: &str) -> CoreResult<Option<String>> {
        Ok(None)
    }
    async fn artifact_bindings_schema(&self, _digest: &str) -> CoreResult<Option<String>> {
        Ok(None)
    }
    async fn artifact_parameter_schema(&self, _digest: &str) -> CoreResult<String> {
        // Empty document = no declared parameters (the validator treats
        // it as an absent schema layer).
        Ok(String::new())
    }
    async fn artifact_shape_ok(&self, _digest: &str) -> CoreResult<bool> {
        Ok(false)
    }
    async fn artifact_lock_digest(&self, _digest: &str) -> CoreResult<Option<String>> {
        Ok(None)
    }
    async fn artifact_lock_file(&self, _digest: &str) -> CoreResult<Option<String>> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl ProfileStore for MemoryStore {
    async fn profile_change_get(&self, _change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(None)
    }
    async fn commit_profile_retirement(
        &self,
        _facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
    async fn attestation_get(
        &self,
        _profile_key: &str,
        _revision: i64,
        _attestation_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::AttestationReplayRow>> {
        Ok(None)
    }
    async fn commit_attestation(
        &self,
        _record: shaula_core::registry::AttestationRecord,
        _actor: Actor,
        _now: i64,
    ) -> CoreResult<Result<shaula_core::registry::AttestationCommit, MutationError>> {
        Ok(Ok(shaula_core::registry::AttestationCommit::Created))
    }
}

#[async_trait::async_trait]
impl AdmissionSupportStore for MemoryStore {
    async fn idempotency_find(
        &self,
        _principal: &str,
        _operation: &str,
        _resource_kind: &str,
        _resource_key: &str,
        _idempotency_key: &str,
        _request_hash: &str,
    ) -> CoreResult<shaula_core::registry::IdempotencyLookup> {
        Ok(shaula_core::registry::IdempotencyLookup::Miss)
    }
    fn diagnostic_sink(
        &self,
    ) -> Option<std::sync::Arc<dyn shaula_core::diagnostics::DiagnosticSink>> {
        None
    }
    async fn diagnostic_read(
        &self,
        _kind: shaula_core::diagnostics::SubjectKind,
        _key: &str,
        _actor: &Actor,
        _now: i64,
    ) -> shaula_core::diagnostics::DiagnosticsResult {
        Err(shaula_core::diagnostics::DiagnosticsReadError::Unavailable)
    }
}
