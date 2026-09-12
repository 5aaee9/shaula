//! `LifecycleStore` port implementation plus the Store helpers shared by
//! the idempotency and handoff flows.

use async_trait::async_trait;

use shaula_core::error::{CoreError, CoreResult};

use super::core_err;
use super::lifecycle_support::map_generation;
use super::SqliteControlPlane;
use crate::entities::{
    fleet::{fleet_revision_pool_members, fleets},
    lifecycle::runner_generations,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[async_trait]
impl shaula_core::registry::LifecycleStore for SqliteControlPlane {
    async fn generation_admit_pool(
        &self,
        mut record: shaula_core::registry::GenerationRecord,
        guard: &shaula_core::registry::FleetRuntimeGuard,
    ) -> CoreResult<Option<shaula_core::template_pool::PoolGenerationAdmission>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let Some(fleet) = fleets::Entity::find_by_id(record.fleet_key.clone())
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
        else {
            tx.rollback().await.ok();
            return Ok(None);
        };
        if fleet.incarnation != guard.incarnation
            || fleet.desired_revision != guard.desired_revision
            || fleet.mutation_fence != guard.mutation_fence
            || fleet.deletion_marker
            || fleet.tombstone
        {
            tx.rollback().await.ok();
            return Ok(None);
        }
        if let Some(existing) = runner_generations::Entity::find_by_id(record.id.clone())
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
        {
            record = map_generation(existing);
            let inputs = if let Some(key) = record.pool_member_key.as_deref() {
                fleet_revision_pool_members::Entity::find()
                    .filter(fleet_revision_pool_members::Column::FleetKey.eq(&record.fleet_key))
                    .filter(
                        fleet_revision_pool_members::Column::FleetRevision
                            .eq(record.fleet_revision),
                    )
                    .filter(fleet_revision_pool_members::Column::MemberKey.eq(key))
                    .one(&tx)
                    .await
                    .map_err(|e| core_err(e.into()))?
                    .and_then(|m| serde_json::from_str(&m.template_inputs_json).ok())
                    .unwrap_or_default()
            } else {
                serde_json::Map::new()
            };
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(Some(shaula_core::template_pool::PoolGenerationAdmission {
                generation: record,
                template_inputs: inputs,
            }));
        }
        let members = fleet_revision_pool_members::Entity::find()
            .filter(fleet_revision_pool_members::Column::FleetKey.eq(&record.fleet_key))
            .filter(fleet_revision_pool_members::Column::FleetRevision.eq(record.fleet_revision))
            .all(&tx)
            .await
            .map_err(|e| core_err(e.into()))?;
        if members.is_empty() {
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(None);
        }
        if let Some(revision_row) = crate::entities::fleet::fleet_revisions::Entity::find()
            .filter(crate::entities::fleet::fleet_revisions::Column::FleetKey.eq(&record.fleet_key))
            .filter(
                crate::entities::fleet::fleet_revisions::Column::Revision.eq(record.fleet_revision),
            )
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
        {
            if let Ok(spec) =
                serde_json::from_str::<shaula_core::fleet::FleetSpec>(&revision_row.spec_json)
            {
                let occupancy = runner_generations::Entity::find()
                    .filter(runner_generations::Column::FleetKey.eq(&record.fleet_key))
                    .all(&tx)
                    .await
                    .map_err(|e| core_err(e.into()))?
                    .iter()
                    .filter(|g| g.state != "Destroyed")
                    .count() as i64;
                if occupancy >= spec.capacity.max_runners {
                    tx.rollback().await.map_err(|e| core_err(e.into()))?;
                    return Ok(None);
                }
            }
        }
        let generations = runner_generations::Entity::find()
            .filter(runner_generations::Column::FleetKey.eq(&record.fleet_key))
            .all(&tx)
            .await
            .map_err(|e| core_err(e.into()))?;
        let mut eligible = Vec::new();
        let mut member_at_capacity = false;
        for member in members {
            let used = generations
                .iter()
                .filter(|g| {
                    g.state != "Destroyed"
                        && g.pool_member_key.as_deref() == Some(member.member_key.as_str())
                })
                .count() as i64;
            if member.max_runners.is_none_or(|cap| used < cap) {
                eligible.push((member, used));
            } else {
                member_at_capacity = true;
            }
        }
        // Backpressure keeps the configured pool contract intact when any
        // route is unavailable. Redistribute explicitly permits a fresh
        // weighted draw over the currently eligible subset.
        if member_at_capacity {
            let policy = crate::entities::fleet::fleet_revisions::Entity::find()
                .filter(
                    crate::entities::fleet::fleet_revisions::Column::FleetKey.eq(&record.fleet_key),
                )
                .filter(
                    crate::entities::fleet::fleet_revisions::Column::Revision
                        .eq(record.fleet_revision),
                )
                .one(&tx)
                .await
                .map_err(|e| core_err(e.into()))?
                .and_then(|row| {
                    serde_json::from_str::<shaula_core::fleet::FleetSpec>(&row.spec_json).ok()
                })
                .map(|spec| {
                    spec.template_pool
                        .map(|pool| pool.failure_policy)
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            if policy == shaula_core::template_pool::PoolFailurePolicy::Backpressure {
                tx.rollback().await.map_err(|e| core_err(e.into()))?;
                return Ok(None);
            }
        }
        if eligible.is_empty() {
            tx.rollback().await.map_err(|e| core_err(e.into()))?;
            return Ok(None);
        }
        let weights: Vec<u32> = eligible
            .iter()
            .map(|(member, _)| member.weight as u32)
            .collect();
        let index = loop {
            if let Some(index) = shaula_core::template_pool::weighted_member_index(
                &weights,
                uuid::Uuid::new_v4().as_u128(),
            ) {
                break index;
            }
        };
        let selected = eligible
            .into_iter()
            .nth(index)
            .ok_or_else(|| {
                core_err(crate::store::StoreError::Corrupt(
                    "weighted pool selection index missing".into(),
                ))
            })?
            .0;
        record.pool_member_key = Some(selected.member_key.clone());
        record.template_profile_key = selected.template_profile_key.clone();
        record.template_revision = selected.template_revision;
        record.template_artifact_digest = selected.template_artifact_digest.clone();
        record.attestation_id = selected.template_attestation_id.clone();
        record.inputs_digest = selected.inputs_digest.clone();
        crate::Store::generation_insert_on(&tx, record.clone())
            .await
            .map_err(core_err)?;
        let template_inputs = serde_json::from_str(&selected.template_inputs_json)
            .map_err(|e| core_err(crate::store::StoreError::Corrupt(e.to_string())))?;
        tx.commit().await.map_err(|e| core_err(e.into()))?;
        Ok(Some(shaula_core::template_pool::PoolGenerationAdmission {
            generation: record,
            template_inputs,
        }))
    }
    async fn operation_record_bootstrap_starting(
        &self,
        provenance: &shaula_core::ports::PlanProvenance,
        now: i64,
    ) -> CoreResult<()> {
        if self
            .store
            .operation_bootstrap_starting(provenance, now)
            .await
            .map_err(core_err)?
        {
            Ok(())
        } else {
            Err(CoreError::new(
                shaula_core::error::ReasonCode::OwnershipConflict,
                "container bootstrap already consumed or Create authority changed",
            ))
        }
    }

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
                owned_scale_set_id: s.owned_scale_set_id,
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

    async fn generation_set_forgejo_runner(
        &self,
        id: &str,
        runner_id: i64,
        runner_uuid: &str,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .generation_set_forgejo_runner(id, runner_id, runner_uuid, now)
            .await
            .map_err(core_err)
    }

    async fn generation_forgejo_runner(&self, id: &str) -> CoreResult<Option<(i64, String)>> {
        self.store
            .generation_forgejo_runner(id)
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
        self.record_apply_starting_impl(provenance, saved_plan_path, now)
            .await
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
