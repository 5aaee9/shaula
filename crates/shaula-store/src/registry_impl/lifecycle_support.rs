//! Mapping and fence helpers shared by the lifecycle and fleet commit
//! paths, split from `lifecycle_impl.rs` to keep every file within the
//! 400-line limit (AGENTS.md).

use shaula_core::registry::{MutationError, MutationFacts};

impl super::SqliteControlPlane {
    pub(super) async fn generations_occupancy_impl(
        &self,
        fleet_key: &str,
    ) -> shaula_core::error::CoreResult<i64> {
        use shaula_core::lifecycle::GenerationState;
        Ok(self
            .store
            .generations_for_fleet(fleet_key)
            .await
            .map_err(super::core_err)?
            .iter()
            .filter(|g| {
                GenerationState::from_str_repr(&g.state).is_ok_and(|s| s.counts_occupancy())
            })
            .count() as i64)
    }

    pub(super) async fn capacity_counters_impl(
        &self,
        fleet_key: &str,
    ) -> shaula_core::error::CoreResult<(i64, i64)> {
        use shaula_core::lifecycle::GenerationState;
        let mut effective = 0i64;
        let mut occupancy = 0i64;
        for generation in self
            .store
            .generations_for_fleet(fleet_key)
            .await
            .map_err(super::core_err)?
        {
            if let Ok(state) = GenerationState::from_str_repr(&generation.state) {
                effective += i64::from(state.counts_effective());
                occupancy += i64::from(state.counts_occupancy());
            }
        }
        Ok((effective, occupancy))
    }
}

pub(crate) fn map_generation(
    g: crate::entities::lifecycle::runner_generations::Model,
) -> shaula_core::registry::GenerationRecord {
    shaula_core::registry::GenerationRecord {
        id: g.id,
        fleet_key: g.fleet_key,
        runner_name: g.runner_name,
        generation_name: g.generation_name,
        fleet_revision: g.fleet_revision,
        template_profile_key: g.template_profile_key,
        template_revision: g.template_revision,
        template_artifact_digest: g.template_artifact_digest,
        attestation_id: g.attestation_id,
        inputs_digest: g.inputs_digest,
        state: shaula_core::lifecycle::GenerationState::from_str_repr(&g.state)
            .unwrap_or(shaula_core::lifecycle::GenerationState::Quarantined),
        github_runner_id: g.github_runner_id,
        workspace_path: g.workspace_path,
        created_at: g.created_at,
        updated_at: g.updated_at,
    }
}

/// Maps a lost mutation-fence race onto the HTTP status contract:
/// decommissioning fleets map to Gone, stale revisions to 412.
pub(crate) fn fence_conflict(
    facts: &MutationFacts,
    error: &crate::store::StoreError,
) -> MutationError {
    let summary = error.to_string();
    if summary.contains(":decommissioning") && facts.change.kind != "Decommission" {
        return MutationError::Gone {
            tombstone: facts.resource_key.clone(),
        };
    }
    MutationError::PreconditionFailed {
        current: (facts.incarnation.clone(), facts.revision.saturating_sub(1)),
    }
}

use crate::entities::shared::idempotency_records;
use crate::store::Store;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

impl Store {
    pub(crate) async fn idempotency_find_by_key(
        &self,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
    ) -> crate::store::StoreResult<Option<idempotency_records::Model>> {
        idempotency_records::Entity::find()
            .filter(idempotency_records::Column::ResourceKind.eq(resource_kind))
            .filter(idempotency_records::Column::ResourceKey.eq(resource_key))
            .filter(idempotency_records::Column::IdempotencyKey.eq(idempotency_key))
            .one(self.connection())
            .await
            .map_err(crate::store::StoreError::from)
    }
}
