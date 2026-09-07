//! Mapping and fence helpers shared by the lifecycle and fleet commit
//! paths, split from `lifecycle_impl.rs` to keep every file within the
//! 400-line limit (AGENTS.md).

use shaula_core::registry::{MutationError, MutationFacts};

use super::core_err;
use super::SqliteControlPlane;

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

impl SqliteControlPlane {
    /// Applies GitHub identity/access validation outcome for an Auth
    /// Candidate (staged activation). Called by the phase-3 validator and
    /// available to integration tests.
    pub async fn auth_apply_validation(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
    ) -> shaula_core::error::CoreResult<()> {
        self.store
            .auth_scan_apply(key, revision, accepted, reason, now)
            .await
            .map_err(core_err)
    }
}
