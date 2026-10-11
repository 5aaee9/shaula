use super::{core_err, SqliteControlPlane};
use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    Actor, AdmissionSupportStore, ArtifactStore, AuthHandoffRow, CapacityStore, ChangeView,
    FleetHead, FleetRevisionRow, FleetStore, HandoffStore, KeyPage, MutationError, MutationFacts,
};

#[async_trait]
impl FleetStore for SqliteControlPlane {
    async fn fleet_get(&self, key: &str) -> CoreResult<Option<FleetHead>> {
        self.fleet_get_read(key).await
    }
    async fn fleet_list(&self, _: &Actor, page: KeyPage) -> CoreResult<Vec<(String, i64, String)>> {
        self.fleet_list_read(&page).await
    }
    async fn fleet_count(&self) -> CoreResult<usize> {
        Ok(self.store.fleet_list().await.map_err(core_err)?.len())
    }
    async fn fleet_revision_latest(&self, key: &str) -> CoreResult<Option<FleetRevisionRow>> {
        self.fleet_revision_latest_read(key).await
    }
    async fn fleet_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        self.fleet_change_get_read(change_id).await
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
    async fn commit_decommission(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_decommission_impl(facts).await
    }
}

#[async_trait]
impl CapacityStore for SqliteControlPlane {
    async fn demand_get(&self, fleet_key: &str) -> CoreResult<Option<i64>> {
        Ok(self
            .store
            .demand_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(|d| d.total_assigned_jobs))
    }
    async fn demand_with_time(&self, key: &str) -> CoreResult<Option<(i64, Option<i64>)>> {
        Ok(self
            .store
            .demand_get(key)
            .await
            .map_err(core_err)?
            .map(|d| (d.total_assigned_jobs, Some(d.updated_at))))
    }
    async fn capacity_counters(&self, fleet_key: &str) -> CoreResult<(i64, i64)> {
        self.capacity_counters_impl(fleet_key).await
    }
    async fn generations_occupancy(&self, fleet_key: &str) -> CoreResult<i64> {
        self.generations_occupancy_impl(fleet_key).await
    }
    async fn generation_lookup(
        &self,
        id: &str,
    ) -> CoreResult<Option<shaula_core::registry::GenerationRecord>> {
        self.generation_lookup_read(id).await
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
}

#[async_trait]
impl HandoffStore for SqliteControlPlane {
    async fn handoff_get(&self, fleet_key: &str) -> CoreResult<Option<AuthHandoffRow>> {
        self.handoff_get_read(fleet_key).await
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
}

#[async_trait]
impl ArtifactStore for SqliteControlPlane {
    async fn ensure_artifact_cached(&self, digest: &str) -> CoreResult<()> {
        self.ensure_artifact_cached_read(digest).await
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
}

#[async_trait]
impl AdmissionSupportStore for SqliteControlPlane {
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
    fn diagnostic_sink(
        &self,
    ) -> Option<std::sync::Arc<dyn shaula_core::diagnostics::DiagnosticSink>> {
        Some(self.store.diagnostics.clone())
    }
    async fn diagnostic_read(
        &self,
        kind: shaula_core::diagnostics::SubjectKind,
        key: &str,
        actor: &shaula_core::registry::Actor,
        now: i64,
    ) -> shaula_core::diagnostics::DiagnosticsResult {
        use shaula_core::diagnostics::DiagnosticsReadPort;
        self.store.diagnostics(kind, key, actor, now).await
    }
}
