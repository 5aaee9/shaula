//! `LifecycleStore` port implementation plus the Store helpers shared by
//! the idempotency and handoff flows.

use async_trait::async_trait;

use shaula_core::error::{CoreError, CoreResult};

use super::core_err;
use super::lifecycle_support::map_generation;
use super::SqliteControlPlane;

#[async_trait]
impl shaula_core::registry::LifecycleStore for SqliteControlPlane {
    async fn session_close_authorize(
        &self,
        fleet_key: &str,
        guard: &shaula_core::registry::FleetRuntimeGuard,
    ) -> CoreResult<bool> {
        self.store
            .session_close_authorize(fleet_key, guard)
            .await
            .map_err(core_err)
    }

    async fn session_authorize(
        &self,
        fleet_key: &str,
        guard: &shaula_core::registry::FleetRuntimeGuard,
        auth_context: &shaula_core::auth_context::ResolvedAuthContext,
    ) -> CoreResult<bool> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let result = self
            .store
            .runtime_auth_guard_tx(&tx, fleet_key, guard, auth_context)
            .await
            .map_err(core_err)?;
        tx.commit().await.map_err(|e| core_err(e.into()))?;
        Ok(result)
    }

    async fn session_epoch(&self, fleet_key: &str) -> CoreResult<Option<i64>> {
        Ok(self
            .store
            .session_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(|s| s.epoch))
    }

    async fn session_install(
        &self,
        fleet_key: &str,
        install: &shaula_core::registry::SessionInstall,
        now: i64,
    ) -> CoreResult<Option<i64>> {
        self.store
            .session_install(fleet_key, install, now)
            .await
            .map_err(core_err)
    }

    async fn session_get(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::PersistedSession>> {
        self.store.session_handle(fleet_key).await.map_err(core_err)
    }

    async fn session_clear(
        &self,
        fleet_key: &str,
        expected_epoch: i64,
        now: i64,
    ) -> CoreResult<bool> {
        self.store
            .session_clear(fleet_key, expected_epoch, now)
            .await
            .map_err(core_err)
    }

    async fn demand_snapshot(
        &self,
        fleet_key: &str,
        total_assigned_jobs: i64,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .demand_snapshot(fleet_key, total_assigned_jobs, now)
            .await
            .map_err(core_err)
    }

    async fn scale_set_get(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::ScaleSetRow>> {
        Ok(self
            .store
            .scale_set_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(|s| shaula_core::registry::ScaleSetRow {
                fleet_key: s.fleet_key,
                scale_set_id: s.scale_set_id,
                name: s.name,
                runner_group: s.runner_group,
                fingerprint: s.fingerprint,
                state: s.state,
                attempt_id: s.attempt_id,
                now: s.updated_at,
            }))
    }

    async fn scale_set_upsert(&self, row: shaula_core::registry::ScaleSetRow) -> CoreResult<()> {
        self.store.scale_set_upsert(row).await.map_err(core_err)
    }

    async fn generation_insert(
        &self,
        record: shaula_core::registry::GenerationRecord,
    ) -> CoreResult<()> {
        self.store.generation_insert(record).await.map_err(core_err)
    }

    async fn generation_get(
        &self,
        id: &str,
    ) -> CoreResult<Option<shaula_core::registry::GenerationRecord>> {
        Ok(self
            .store
            .generation_get(id)
            .await
            .map_err(core_err)?
            .map(map_generation))
    }

    async fn generations_for_fleet(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Vec<shaula_core::registry::GenerationRecord>> {
        Ok(self
            .store
            .generations_for_fleet(fleet_key)
            .await
            .map_err(core_err)?
            .into_iter()
            .map(map_generation)
            .collect())
    }

    async fn generation_advance(
        &self,
        id: &str,
        next: shaula_core::lifecycle::GenerationState,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .generation_advance(id, next, None, now)
            .await
            .map_err(core_err)?;
        Ok(())
    }

    async fn generation_set_github_runner(
        &self,
        id: &str,
        runner_id: i64,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .generation_set_jit(id, "JITReady", Some(runner_id), now)
            .await
            .map_err(core_err)
    }

    async fn generation_set_jit_phase(&self, id: &str, phase: &str, now: i64) -> CoreResult<()> {
        self.store
            .generation_set_jit(id, phase, None, now)
            .await
            .map_err(core_err)
    }

    async fn operation_insert(
        &self,
        insert: shaula_core::registry::OperationInsert,
    ) -> CoreResult<()> {
        self.store.operation_insert(insert).await.map_err(core_err)
    }

    async fn generation_set_result(
        &self,
        id: &str,
        result_json: &str,
        digest: &str,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .generation_set_result(id, result_json, digest, now)
            .await
            .map_err(core_err)
    }

    async fn operation_record_apply_starting(
        &self,
        provenance: &shaula_core::ports::PlanProvenance,
        saved_plan_path: &str,
        now: i64,
    ) -> CoreResult<()> {
        // The fence and the durable record share ONE transaction: a
        // DELETE/newer PUT that lands before the insert refuses it, so
        // the check and the spawn-eligibility record can never diverge
        // (F05; spec 0002 §8.334).
        let tx = self.store.begin().await.map_err(core_err)?;
        let kind = match provenance.intent {
            shaula_core::plan::PlanIntent::Create => "Create",
            shaula_core::plan::PlanIntent::Destroy => "Destroy",
        };
        let provenance_json = serde_json::to_string(provenance).map_err(|e| {
            CoreError::new(
                shaula_core::error::ReasonCode::Internal,
                format!("apply provenance serialize failed: {e}"),
            )
        })?;
        let insert = shaula_core::registry::OperationInsert {
            id: provenance.attempt_id.clone(),
            generation_id: provenance.generation_id.clone(),
            kind: kind.to_string(),
            state: "ApplyStarting".to_string(),
            provenance_json: Some(provenance_json),
            saved_plan_path: Some(saved_plan_path.to_string()),
            saved_plan_digest: Some(provenance.saved_plan_digest.clone()),
            now,
        };
        match self
            .store
            .operation_apply_starting_tx(&tx, insert)
            .await
            .map_err(core_err)?
        {
            Ok(()) => {
                tx.commit().await.map_err(|e| {
                    CoreError::new(
                        shaula_core::error::ReasonCode::StorageUnavailable,
                        format!("apply-start transaction commit failed: {e}"),
                    )
                })?;
                Ok(())
            }
            Err(summary) => {
                // Fence refusal: drop the transaction (rollback) and
                // surface a conflict the runtime classifies as "apply
                // refused before spawn".
                Err(CoreError::new(
                    shaula_core::error::ReasonCode::OwnershipConflict,
                    summary,
                ))
            }
        }
    }

    async fn generation_state_identity(&self, id: &str) -> CoreResult<Option<(String, u64)>> {
        self.store
            .generation_state_identity(id)
            .await
            .map_err(core_err)
    }

    async fn generation_destroy_attempted(&self, id: &str) -> CoreResult<bool> {
        self.store
            .generation_destroy_attempted(id)
            .await
            .map_err(core_err)
    }

    async fn operation_original_provenance(
        &self,
        generation_id: &str,
    ) -> CoreResult<Option<shaula_core::ports::PlanProvenance>> {
        self.store
            .operation_provenance_get(generation_id, "Create")
            .await
            .map_err(core_err)?
            .map(|json| {
                serde_json::from_str(&json).map_err(|e| {
                    shaula_core::error::CoreError::new(
                        shaula_core::error::ReasonCode::StorageUnavailable,
                        format!("stored provenance unreadable: {e}"),
                    )
                })
            })
            .transpose()
    }

    async fn operation_update_state(&self, id: &str, state: &str, now: i64) -> CoreResult<()> {
        self.store
            .operation_update_state(id, state, now)
            .await
            .map_err(core_err)
    }

    async fn operations_open_for_generation(
        &self,
        generation_id: &str,
    ) -> CoreResult<Vec<shaula_core::registry::OperationRow>> {
        Ok(self
            .store
            .operations_open_for_generation(generation_id)
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|op| shaula_core::registry::OperationRow {
                id: op.id,
                generation_id: op.generation_id,
                kind: op.kind,
                state: op.state,
                attempts: op.attempts,
                saved_plan_digest: op.saved_plan_digest,
            })
            .collect())
    }

    async fn fleet_set_observed(
        &self,
        key: &str,
        observation: &shaula_core::registry::FleetObservation,
        now: i64,
    ) -> CoreResult<bool> {
        self.store
            .fleet_set_observed(key, observation, now)
            .await
            .map_err(core_err)
    }

    async fn fleet_set_tombstone(&self, key: &str, now: i64) -> CoreResult<()> {
        self.store
            .fleet_set_tombstone(key, now)
            .await
            .map_err(core_err)
    }

    async fn change_update(
        &self,
        id: &str,
        state: &str,
        reason: Option<&str>,
        next_retry_at: Option<i64>,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .change_update(id, state, reason, next_retry_at, now)
            .await
            .map_err(core_err)
    }

    async fn profile_change_update(
        &self,
        id: &str,
        state: &str,
        reason: Option<&str>,
        next_retry_at: Option<i64>,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .profile_change_update(id, state, reason, next_retry_at, now)
            .await
            .map_err(core_err)
    }

    async fn fleet_head_guard(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::FleetHeadGuard>> {
        self.store
            .fleet_head_guard(fleet_key)
            .await
            .map_err(core_err)
    }
}
