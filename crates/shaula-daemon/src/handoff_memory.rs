//! In-memory ControlPlaneStore test double for handoff tests.

use shaula_core::error::CoreResult;
use shaula_core::registry::{Actor, ControlPlaneStore, MutationError};

#[derive(Default)]
pub struct MemoryStore {
    pub handoffs: tokio::sync::Mutex<
        std::collections::HashMap<String, shaula_core::registry::AuthHandoffRow>,
    >,
}

#[async_trait::async_trait]
impl ControlPlaneStore for MemoryStore {
    async fn commit_profile_retirement(
        &self,
        _facts: shaula_core::registry::MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
    async fn fleet_get(&self, _key: &str) -> CoreResult<Option<shaula_core::registry::FleetHead>> {
        Ok(None)
    }
    async fn fleet_list(&self, _actor: &Actor) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(Vec::new())
    }
    async fn fleet_count(&self) -> CoreResult<usize> {
        Ok(0)
    }
    async fn fleet_revision_latest(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::registry::FleetRevisionRow>> {
        Ok(None)
    }
    async fn generations_occupancy(&self, _fleet_key: &str) -> CoreResult<i64> {
        Ok(0)
    }
    async fn capacity_counters(&self, _fleet_key: &str) -> CoreResult<(i64, i64)> {
        Ok((0, 0))
    }
    async fn handoff_get(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::AuthHandoffRow>> {
        Ok(self.handoffs.lock().await.get(fleet_key).cloned())
    }
    async fn template_profile_get(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::registry::ProfileHead>> {
        Ok(None)
    }
    async fn template_profile_keys(&self) -> CoreResult<Vec<String>> {
        Ok(Vec::new())
    }
    async fn template_revision_get(
        &self,
        _key: &str,
        _revision: i64,
    ) -> CoreResult<Option<shaula_core::registry::TemplateRevisionRow>> {
        Ok(None)
    }
    async fn auth_profile_get(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::registry::ProfileHead>> {
        Ok(None)
    }
    async fn auth_profile_keys(&self) -> CoreResult<Vec<String>> {
        Ok(Vec::new())
    }
    async fn auth_revision_get(
        &self,
        _key: &str,
        _revision: i64,
    ) -> CoreResult<Option<shaula_core::registry::AuthRevisionRow>> {
        Ok(None)
    }
    async fn auth_revision_active(
        &self,
        _key: &str,
    ) -> CoreResult<Option<shaula_core::registry::AuthRevisionRow>> {
        Ok(None)
    }
    async fn auth_apply_validation(
        &self,
        _key: &str,
        _revision: i64,
        _accepted: bool,
        _reason: Option<&str>,
        _now: i64,
    ) -> CoreResult<()> {
        Ok(())
    }
    async fn auth_credential_bytes(
        &self,
        _key: &str,
        _revision: i64,
    ) -> CoreResult<Option<Vec<u8>>> {
        Ok(None)
    }
    async fn template_protected_bindings(
        &self,
        _key: &str,
        _revision: i64,
    ) -> CoreResult<Option<(String, String)>> {
        Ok(None)
    }
    async fn demand_get(&self, _fleet_key: &str) -> CoreResult<Option<i64>> {
        Ok(None)
    }
    async fn handoff_acknowledge(
        &self,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
    ) -> CoreResult<()> {
        if let Some(row) = self.handoffs.lock().await.get_mut(fleet_key) {
            row.observed = Some((profile_key.to_string(), revision));
            row.state = "Observed".into();
        }
        Ok(())
    }
    async fn handoff_mark_blocked(
        &self,
        fleet_key: &str,
        reason: &str,
        retry_at: i64,
    ) -> CoreResult<()> {
        if let Some(row) = self.handoffs.lock().await.get_mut(fleet_key) {
            row.state = "Blocked".into();
            row.blocked_reason = Some(reason.to_string());
            row.retry_at = Some(retry_at);
        }
        Ok(())
    }
    async fn fleet_change_get(
        &self,
        _change_id: &str,
    ) -> CoreResult<Option<shaula_core::registry::ChangeView>> {
        Ok(None)
    }
    async fn profile_change_get(
        &self,
        _change_id: &str,
    ) -> CoreResult<Option<shaula_core::registry::ChangeView>> {
        Ok(None)
    }
    async fn idempotency_find(
        &self,
        _resource_kind: &str,
        _resource_key: &str,
        _idempotency_key: &str,
        _request_hash: &str,
    ) -> CoreResult<shaula_core::registry::IdempotencyLookup> {
        Ok(shaula_core::registry::IdempotencyLookup::Miss)
    }
    async fn artifact_manifest(&self, _digest: &str) -> CoreResult<Option<String>> {
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
    async fn commit_fleet_mutation(
        &self,
        _facts: shaula_core::registry::MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Ok(()))
    }
    async fn commit_fleet_noop(
        &self,
        _key: &str,
        _incarnation: &str,
        _revision: i64,
        _actor: &str,
        _idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        _now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Ok(()))
    }
    async fn commit_decommission(
        &self,
        _facts: shaula_core::registry::MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Ok(()))
    }
    async fn commit_template_revision(
        &self,
        _facts: shaula_core::registry::MutationFacts,
        _extra: (String, String, String, String),
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Ok(()))
    }
    async fn commit_template_noop(
        &self,
        _key: &str,
        _incarnation: &str,
        _revision: i64,
        _actor: &str,
        _idempotency: Option<(String, String)>,
        _now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Ok(()))
    }
    async fn commit_auth_revision(
        &self,
        _facts: shaula_core::registry::MutationFacts,
        _credential: shaula_core::registry::AuthRevisionRow,
        _secret_bytes: &[u8],
    ) -> CoreResult<Result<(), shaula_core::registry::MutationError>> {
        Ok(Ok(()))
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
        _actor: String,
        _now: i64,
    ) -> CoreResult<
        Result<shaula_core::registry::AttestationCommit, shaula_core::registry::MutationError>,
    > {
        Ok(Ok(shaula_core::registry::AttestationCommit::Created))
    }
}
