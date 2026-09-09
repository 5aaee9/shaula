use super::mapping::{
    auth_handoff_row, auth_profile_head, auth_row, fleet_change_row, fleet_head,
    fleet_revision_row, profile_change_row, template_row, tpl_profile_head,
};
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
        Ok(self
            .store
            .fleet_get(key)
            .await
            .map_err(core_err)?
            .map(fleet_head))
    }
    async fn fleet_list(
        &self,
        _actor: &shaula_core::registry::Actor,
    ) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(self
            .store
            .fleet_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|f| (f.key, f.desired_revision, f.phase))
            .collect())
    }
    async fn fleet_count(&self) -> CoreResult<usize> {
        Ok(self.store.fleet_list().await.map_err(core_err)?.len())
    }
    async fn fleet_revision_latest(&self, key: &str) -> CoreResult<Option<FleetRevisionRow>> {
        Ok(self
            .store
            .fleet_revision_latest(key)
            .await
            .map_err(core_err)?
            .map(fleet_revision_row))
    }
    async fn generations_occupancy(&self, fleet_key: &str) -> CoreResult<i64> {
        self.generations_occupancy_impl(fleet_key).await
    }
    async fn capacity_counters(&self, fleet_key: &str) -> CoreResult<(i64, i64)> {
        self.capacity_counters_impl(fleet_key).await
    }
    async fn handoff_get(&self, fleet_key: &str) -> CoreResult<Option<AuthHandoffRow>> {
        Ok(self
            .store
            .handoff_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(auth_handoff_row))
    }
    async fn template_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        Ok(self
            .store
            .template_profile_get(key)
            .await
            .map_err(core_err)?
            .map(tpl_profile_head))
    }
    async fn template_profile_keys(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .store
            .template_profiles_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| p.key)
            .collect())
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
        Ok(self
            .store
            .template_revision_get(key, revision)
            .await
            .map_err(core_err)?
            .map(template_row))
    }
    async fn auth_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        Ok(self
            .store
            .auth_profile_get(key)
            .await
            .map_err(core_err)?
            .map(auth_profile_head))
    }
    async fn auth_profile_keys(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .store
            .auth_profiles_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| p.key)
            .collect())
    }
    async fn auth_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<AuthRevisionRow>> {
        Ok(self
            .store
            .auth_revision_get(key, revision)
            .await
            .map_err(core_err)?
            .map(auth_row))
    }
    async fn auth_revision_active(&self, key: &str) -> CoreResult<Option<AuthRevisionRow>> {
        Ok(self
            .store
            .auth_revision_active(key)
            .await
            .map_err(core_err)?
            .map(auth_row))
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
        Ok(self
            .store
            .change_get(change_id)
            .await
            .map_err(core_err)?
            .map(fleet_change_row))
    }
    async fn profile_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(self
            .store
            .profile_change_get(change_id)
            .await
            .map_err(core_err)?
            .map(profile_change_row))
    }
    async fn idempotency_find(
        &self,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<shaula_core::registry::IdempotencyLookup> {
        let Some(record) = self
            .store
            .idempotency_find_by_key(resource_kind, resource_key, idempotency_key)
            .await
            .map_err(core_err)?
        else {
            return Ok(shaula_core::registry::IdempotencyLookup::Miss);
        };
        if record.request_hash != request_hash {
            return Ok(shaula_core::registry::IdempotencyLookup::Conflict);
        }
        Ok(match record.response_body {
            Some(body) => shaula_core::registry::IdempotencyLookup::Replay(body),
            None => shaula_core::registry::IdempotencyLookup::Miss,
        })
    }
    async fn artifact_manifest(&self, digest: &str) -> CoreResult<Option<String>> {
        self.artifact_manifest_impl(digest).await
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
        actor: &str,
        idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_fleet_noop_impl(key, incarnation, revision, actor, idempotency, now)
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
        actor: &str,
        idempotency: Option<(String, String)>,
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
    async fn commit_attestation(
        &self,
        record: AttestationRecord,
        actor: String,
        now: i64,
    ) -> CoreResult<Result<AttestationCommit, MutationError>> {
        self.commit_attestation_impl(record, actor, now).await
    }
}
