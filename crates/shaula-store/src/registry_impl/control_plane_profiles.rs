//! Template, Auth and cross-kind profile ports of the SQLite control plane.

use super::{core_err, SqliteControlPlane};
use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    AttestationCommit, AttestationRecord, AuthProfileStore, AuthRevisionRow, ChangeView, KeyPage,
    MutationError, MutationFacts, ProfileHead, ProfileStore, TemplateRevisionRow, TemplateStore,
};

#[async_trait]
impl TemplateStore for SqliteControlPlane {
    async fn template_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        self.template_profile_get_read(key).await
    }
    async fn template_profile_keys(&self, page: KeyPage) -> CoreResult<Vec<String>> {
        self.template_profile_keys_read(&page).await
    }
    async fn template_references_in_use(&self, key: &str, now: i64) -> CoreResult<Option<bool>> {
        super::retirement_refs::in_use(self.store.connection(), key, true, now)
            .await
            .map(Some)
            .map_err(super::core_err)
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
    async fn template_protected_bindings(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<(String, String)>> {
        self.template_protected_bindings_read(key, revision).await
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
}

#[async_trait]
impl AuthProfileStore for SqliteControlPlane {
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
    async fn auth_credential_bytes(&self, key: &str, revision: i64) -> CoreResult<Option<Vec<u8>>> {
        self.auth_credential_bytes_read(key, revision).await
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
    async fn commit_auth_policy_update(
        &self,
        facts: MutationFacts,
        base_revision: i64,
        policy_json: String,
    ) -> CoreResult<Result<shaula_core::registry::MutationAccepted, MutationError>> {
        self.commit_auth_policy_update_impl(facts, base_revision, policy_json)
            .await
    }
}

#[async_trait]
impl ProfileStore for SqliteControlPlane {
    async fn profile_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        self.profile_change_get_read(change_id).await
    }
    async fn commit_profile_retirement(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_profile_retirement_impl(facts).await
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
        actor: shaula_core::registry::Actor,
        now: i64,
    ) -> CoreResult<Result<AttestationCommit, MutationError>> {
        self.commit_attestation_impl(record, actor, now).await
    }
}
