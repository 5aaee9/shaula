use super::mapping::{
    auth_profile_head, auth_row, fleet_head, fleet_revision_row, template_row, tpl_profile_head,
};
use super::{artifacts, core_err, SqliteControlPlane};
use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    AttestationCommit, AttestationRecord, AuthHandoffRow, AuthRevisionRow, ChangeView,
    ControlPlaneStore, FleetHead, FleetRevisionRow, MutationError, MutationFacts, ProfileHead,
    TemplateRevisionRow,
};

#[async_trait]
impl ControlPlaneStore for SqliteControlPlane {
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
        Ok(self
            .store
            .generations_for_fleet(fleet_key)
            .await
            .map_err(core_err)?
            .iter()
            .filter(|g| {
                shaula_core::lifecycle::GenerationState::from_str_repr(&g.state)
                    .map(|s| s.counts_occupancy())
                    .unwrap_or(false)
            })
            .count() as i64)
    }
    async fn capacity_counters(&self, fleet_key: &str) -> CoreResult<(i64, i64)> {
        let generations = self
            .store
            .generations_for_fleet(fleet_key)
            .await
            .map_err(core_err)?;
        let mut effective = 0i64;
        let mut occupancy = 0i64;
        for generation in &generations {
            if let Ok(state) =
                shaula_core::lifecycle::GenerationState::from_str_repr(&generation.state)
            {
                if state.counts_effective() {
                    effective += 1;
                }
                if state.counts_occupancy() {
                    occupancy += 1;
                }
            }
        }
        Ok((effective, occupancy))
    }
    async fn handoff_get(&self, fleet_key: &str) -> CoreResult<Option<AuthHandoffRow>> {
        Ok(self
            .store
            .handoff_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(|h| AuthHandoffRow {
                fleet_key: h.fleet_key,
                desired: (h.desired_profile_key, h.desired_revision),
                observed: h
                    .observed_profile_key
                    .map(|k| (k, h.observed_revision.unwrap_or_default())),
                state: h.state,
                cleanup_only: h.cleanup_only,
                blocked_reason: h.reason.clone(),
                retry_at: h.next_retry_at,
            }))
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
    async fn auth_apply_validation(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .auth_scan_apply(key, revision, accepted, reason, now)
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
    ) -> CoreResult<()> {
        self.store
            .handoff_acknowledge(fleet_key, profile_key, revision)
            .await
            .map_err(core_err)
    }
    async fn handoff_mark_blocked(
        &self,
        fleet_key: &str,
        reason: &str,
        retry_at: i64,
    ) -> CoreResult<()> {
        self.store
            .handoff_mark_blocked(fleet_key, reason, retry_at)
            .await
            .map_err(core_err)
    }
    async fn fleet_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(self
            .store
            .change_get(change_id)
            .await
            .map_err(core_err)?
            .map(|c| ChangeView {
                id: c.id,
                resource_kind: "fleet".to_string(),
                resource_key: c.fleet_key,
                revision: c.revision,
                kind: c.kind,
                state: c.state,
                reason: c.reason,
            }))
    }
    async fn profile_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(self
            .store
            .profile_change_get(change_id)
            .await
            .map_err(core_err)?
            .map(|c| ChangeView {
                id: c.id,
                resource_kind: c.resource_kind,
                resource_key: c.profile_key,
                revision: c.revision.unwrap_or_default(),
                kind: c.kind,
                state: c.state,
                reason: c.reason,
            }))
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
        artifacts::manifest(&self.artifact_root, digest)
    }
    async fn artifact_parameter_schema(&self, digest: &str) -> CoreResult<String> {
        // Ok(None) only for a malformed digest; missing/corrupt file is
        // already an Err — admission never degrades to "no schema".
        match artifacts::parameter_schema(&self.artifact_root, digest)? {
            Some(text) => Ok(text),
            None => Err(shaula_core::error::CoreError::new(
                shaula_core::error::ReasonCode::StorageUnavailable,
                "artifact digest malformed",
            )),
        }
    }
    async fn artifact_shape_ok(&self, digest: &str) -> CoreResult<bool> {
        Ok(artifacts::shape_ok(&self.artifact_root, digest))
    }
    async fn artifact_lock_digest(&self, digest: &str) -> CoreResult<Option<String>> {
        artifacts::lock_digest(&self.artifact_root, digest)
    }
    async fn artifact_lock_file(&self, digest: &str) -> CoreResult<Option<String>> {
        artifacts::lock_file(&self.artifact_root, digest)
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
    ) -> CoreResult<Result<(), MutationError>> {
        self.commit_template_revision_impl(facts, extra).await
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
