use super::mapping::{
    auth_handoff_row, fleet_change_row, fleet_head, fleet_pool_member_row, fleet_revision_row,
    pool_member_row,
};
use super::{core_err, SqliteControlPlane};
use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    Actor, AdmissionSupportStore, AuthHandoffRow, CapacityStore, ChangeView, FleetHead,
    FleetRevisionRow, FleetStore, HandoffStore, IdempotencyLookup, KeyPage, MutationError,
    MutationFacts,
};

#[async_trait]
impl FleetStore for SqliteControlPlane {
    async fn fleet_get(&self, key: &str) -> CoreResult<Option<FleetHead>> {
        Ok(self
            .store
            .fleet_get(key)
            .await
            .map_err(core_err)?
            .map(fleet_head))
    }
    async fn fleet_list(&self, _: &Actor, page: KeyPage) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(self
            .store
            .fleet_page(&page)
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
        let Some(row) = self
            .store
            .fleet_revision_latest(key)
            .await
            .map_err(core_err)?
        else {
            return Ok(None);
        };
        let mut mapped = fleet_revision_row(row.clone());
        let pool_scope = row
            .template_pool_ref
            .clone()
            .zip(row.template_pool_revision);
        // Shared-pool fleets hydrate their members from the pool revision
        // the fleet revision froze (spec 0037 §4); inline pools keep
        // reading their own member rows.
        mapped.template_pool = if let Some((pool_key, pool_revision)) = &pool_scope {
            self.store
                .template_pool_members(pool_key, *pool_revision)
                .await
                .map_err(core_err)?
                .into_iter()
                .map(pool_member_row)
                .collect::<Result<Vec<_>, _>>()
                .map_err(core_err)?
        } else {
            self.store
                .fleet_revision_pool_members(key, row.revision)
                .await
                .map_err(core_err)?
                .into_iter()
                .map(fleet_pool_member_row)
                .collect::<Result<Vec<_>, _>>()
                .map_err(core_err)?
        };
        Ok(Some(mapped))
    }
    async fn fleet_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(self
            .store
            .change_get(change_id)
            .await
            .map_err(core_err)?
            .map(fleet_change_row))
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
        Ok(self
            .store
            .generation_get(id)
            .await
            .map_err(core_err)?
            .map(super::lifecycle_support::map_generation))
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
        self.store
            .handoff_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(auth_handoff_row)
            .transpose()
            .map_err(core_err)
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
impl AdmissionSupportStore for SqliteControlPlane {
    async fn idempotency_find(
        &self,
        principal: &str,
        operation: &str,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<IdempotencyLookup> {
        if self
            .store
            .idempotency_find_by_key(None, None, resource_kind, resource_key, idempotency_key)
            .await
            .map_err(core_err)?
            .is_some()
        {
            return Ok(IdempotencyLookup::LegacyConflict);
        }
        let Some(record) = self
            .store
            .idempotency_find_by_key(
                Some(principal),
                Some(operation),
                resource_kind,
                resource_key,
                idempotency_key,
            )
            .await
            .map_err(core_err)?
        else {
            return Ok(IdempotencyLookup::Miss);
        };
        if record.request_hash != request_hash {
            return Ok(IdempotencyLookup::Conflict);
        }
        Ok(match record.response_body {
            Some(body) => IdempotencyLookup::Replay(body),
            None => IdempotencyLookup::Miss,
        })
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
        self.store.diagnostics(kind, key, actor, now).await
    }
}
