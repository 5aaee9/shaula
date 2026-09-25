use super::{core_err, SqliteControlPlane};
use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    AttestationCommit, AttestationRecord, AuthHandoffRow, AuthRevisionRow, ChangeView,
    ControlPlaneStore, FleetHead, FleetRevisionRow, MutationError, MutationFacts, ProfileHead,
    TemplateRevisionRow,
};

#[async_trait]
impl ControlPlaneStore for SqliteControlPlane {
    async fn ensure_artifact_cached(&self, digest: &str) -> CoreResult<()> {
        if !self.artifact_available(digest).await? {
            return Err(shaula_core::error::CoreError::new(
                shaula_core::error::ReasonCode::TemplateInvalid,
                "artifact digest is not published",
            ));
        }
        Ok(())
    }
    async fn commit_profile_retirement(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_profile_retirement_impl(facts).await
    }
    async fn fleet_get(&self, key: &str) -> CoreResult<Option<FleetHead>> {
        self.fleet_get_read(key).await
    }
    async fn fleet_list(
        &self,
        _actor: &shaula_core::registry::Actor,
    ) -> CoreResult<Vec<(String, i64, String)>> {
        self.fleet_list_read(_actor).await
    }
    async fn fleet_count(&self) -> CoreResult<usize> {
        Ok(self.store.fleet_list().await.map_err(core_err)?.len())
    }
    async fn fleet_revision_latest(&self, key: &str) -> CoreResult<Option<FleetRevisionRow>> {
        self.fleet_revision_latest_read(key).await
    }
    async fn generation_lookup(
        &self,
        id: &str,
    ) -> CoreResult<Option<shaula_core::registry::GenerationRecord>> {
        self.generation_lookup_read(id).await
    }
    async fn generations_occupancy(&self, fleet_key: &str) -> CoreResult<i64> {
        self.generations_occupancy_impl(fleet_key).await
    }
    async fn capacity_counters(&self, fleet_key: &str) -> CoreResult<(i64, i64)> {
        self.capacity_counters_impl(fleet_key).await
    }
    async fn handoff_get(&self, fleet_key: &str) -> CoreResult<Option<AuthHandoffRow>> {
        self.handoff_get_read(fleet_key).await
    }
    async fn template_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        self.template_profile_get_read(key).await
    }
    async fn template_profile_keys(&self) -> CoreResult<Vec<String>> {
        self.template_profile_keys_read().await
    }
    async fn template_source_get(
        &self,
        key: &str,
    ) -> CoreResult<Option<shaula_core::registry::TemplateSource>> {
        self.store.template_source_get(key).await.map_err(core_err)
    }
    async fn template_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<TemplateRevisionRow>> {
        self.template_revision_get_read(key, revision).await
    }
    async fn auth_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        self.auth_profile_get_read(key).await
    }
    async fn auth_profile_keys(&self) -> CoreResult<Vec<String>> {
        self.auth_profile_keys_read().await
    }
    async fn auth_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<AuthRevisionRow>> {
        self.auth_revision_get_read(key, revision).await
    }
    async fn auth_revision_active(&self, key: &str) -> CoreResult<Option<AuthRevisionRow>> {
        self.auth_revision_active_read(key).await
    }
    async fn auth_apply_validation_v2(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
        promotion: Option<shaula_core::registry::AuthPromotion>,
    ) -> CoreResult<shaula_core::registry::AuthPromotionOutcome> {
        self.store
            .auth_apply_full(key, revision, accepted, reason, now, promotion)
            .await
            .map_err(core_err)
    }
    async fn auth_bindings_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Vec<shaula_core::auth_context::AccountBinding>> {
        self.store
            .auth_bindings_get(key, revision)
            .await
            .map_err(core_err)
    }
    async fn auth_live_dependents(
        &self,
        key: &str,
    ) -> CoreResult<Vec<shaula_core::registry::AuthDependentTarget>> {
        self.store.auth_live_dependents(key).await.map_err(core_err)
    }
    async fn fleet_auth_context_get(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::FleetAuthContextRow>> {
        Ok(self
            .store
            .fleet_auth_context_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(crate::auth_policy_repo::fleet_auth_context_row))
    }
    async fn fleet_auth_context_block(
        &self,
        fleet_key: &str,
        reason: &str,
        retry_at: i64,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .fleet_auth_context_block(fleet_key, reason, retry_at, now)
            .await
            .map_err(core_err)
    }
    async fn auth_credential_bytes(&self, key: &str, revision: i64) -> CoreResult<Option<Vec<u8>>> {
        Ok(self
            .store
            .auth_revision_get(key, revision)
            .await
            .map_err(core_err)?
            .map(|r| r.credential_bytes))
    }
    async fn template_protected_bindings(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<(String, String)>> {
        Ok(self
            .store
            .template_revision_get(key, revision)
            .await
            .map_err(core_err)?
            .and_then(|r| r.bindings_json.clone().zip(r.bindings_digest.clone())))
    }
    async fn demand_get(&self, fleet_key: &str) -> CoreResult<Option<i64>> {
        Ok(self
            .store
            .demand_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(|d| d.total_assigned_jobs))
    }
    async fn handoff_acknowledge(
        &self,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
        context_json: Option<&str>,
        expectation: &shaula_core::registry::AuthHandoffExpectation,
    ) -> CoreResult<shaula_core::registry::FleetContextAck> {
        self.store
            .handoff_acknowledge(fleet_key, profile_key, revision, context_json, expectation)
            .await
            .map_err(core_err)
    }
    async fn handoff_mark_blocked(
        &self,
        fleet_key: &str,
        authority: &(String, i64),
        expectation: &shaula_core::registry::AuthHandoffExpectation,
        reason: &str,
        retry_at: i64,
    ) -> CoreResult<bool> {
        self.store
            .handoff_mark_blocked(fleet_key, authority, expectation, reason, retry_at)
            .await
            .map_err(core_err)
    }
    async fn fleet_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        self.fleet_change_get_read(change_id).await
    }
    async fn profile_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        self.profile_change_get_read(change_id).await
    }
    async fn idempotency_find(
        &self,
        principal: &str,
        operation: &str,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<shaula_core::registry::IdempotencyLookup> {
        self.idempotency_find_read(
            principal,
            operation,
            resource_kind,
            resource_key,
            idempotency_key,
            request_hash,
        )
        .await
    }
    async fn artifact_manifest(&self, digest: &str) -> CoreResult<Option<String>> {
        self.artifact_manifest_impl(digest).await
    }
    async fn artifact_bindings_schema(&self, digest: &str) -> CoreResult<Option<String>> {
        self.artifact_bindings_schema_impl(digest).await
    }
    async fn artifact_parameter_schema(&self, digest: &str) -> CoreResult<String> {
        self.artifact_parameter_schema_impl(digest).await
    }
    async fn artifact_shape_ok(&self, digest: &str) -> CoreResult<bool> {
        self.artifact_shape_impl(digest).await
    }
    async fn artifact_lock_digest(&self, digest: &str) -> CoreResult<Option<String>> {
        self.artifact_lock_digest_impl(digest).await
    }
    async fn artifact_lock_file(&self, digest: &str) -> CoreResult<Option<String>> {
        self.artifact_lock_file_impl(digest).await
    }
    async fn commit_fleet_mutation(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_fleet_mutation_impl(facts).await
    }
    async fn commit_fleet_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_fleet_noop_impl(key, incarnation, revision, actor, idempotency, now)
            .await
    }
    async fn commit_generation_finalize(
        &self,
        generation_id: &str,
        actor: &shaula_core::registry::Actor,
        reason: &str,
        idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_generation_finalize_impl(
            generation_id,
            &shaula_core::auth::new_attempt_id(),
            actor,
            reason,
            idempotency,
            now,
        )
        .await
    }
    async fn commit_decommission(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_decommission_impl(facts).await
    }
    async fn commit_template_revision(
        &self,
        facts: MutationFacts,
        extra: (String, String, String, String),
        source_key: Option<String>,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_revision_impl(facts, extra, source_key)
            .await
    }
    async fn commit_template_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<(String, String, String)>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_noop_impl(key, incarnation, revision, actor, idempotency, now)
            .await
    }
    async fn commit_auth_revision(
        &self,
        facts: MutationFacts,
        credential: AuthRevisionRow,
        secret_bytes: &[u8],
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_auth_revision_impl(facts, credential, secret_bytes)
            .await
    }
    async fn attestation_get(
        &self,
        profile_key: &str,
        revision: i64,
        attestation_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::AttestationReplayRow>> {
        self.attestation_lookup(profile_key, revision, attestation_key)
            .await
    }
    async fn commit_auth_policy_update(
        &self,
        facts: MutationFacts,
        base_revision: i64,
        policy_json: String,
    ) -> CoreResult<Result<shaula_core::registry::MutationAccepted, MutationError>> {
        self.commit_auth_policy_update_impl(facts, base_revision, policy_json)
            .await
    }
    async fn commit_attestation(
        &self,
        record: AttestationRecord,
        actor: shaula_core::registry::Actor,
        now: i64,
    ) -> CoreResult<Result<AttestationCommit, MutationError>> {
        self.commit_attestation_impl(record, actor, now).await
    }

    async fn template_pool_get(
        &self,
        key: &str,
    ) -> CoreResult<Option<shaula_core::template_pool::TemplatePoolHead>> {
        self.template_pool_get_read(key).await
    }
    async fn template_pool_list(&self) -> CoreResult<Vec<(String, i64, String)>> {
        self.template_pool_list_read().await
    }
    async fn template_pool_revision_latest(
        &self,
        key: &str,
    ) -> CoreResult<Option<shaula_core::template_pool::TemplatePoolRevision>> {
        self.template_pool_revision_row(key).await
    }

    async fn commit_template_pool_mutation(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_pool_mutation_impl(facts).await
    }

    async fn commit_template_pool_delete(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_pool_delete_impl(facts).await
    }

    async fn template_pool_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        self.template_pool_change_get_read(change_id).await
    }
}
