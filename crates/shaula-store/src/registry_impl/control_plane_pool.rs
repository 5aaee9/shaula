//! ControlPlaneStore trait methods for shared TemplatePool resources
//! (spec 0037), split out to keep `control_plane_store` within the
//! 400-line module limit.

use super::{core_err, SqliteControlPlane};
use shaula_core::error::CoreResult;
use shaula_core::template_pool::TemplatePoolRevision;

pub(crate) fn pool_member_row(
    m: crate::entities::template_pool::template_pool_members::Model,
) -> shaula_core::template_pool::ResolvedTemplatePoolMember {
    shaula_core::template_pool::ResolvedTemplatePoolMember {
        key: m.member_key,
        template_profile_key: m.template_profile_key,
        template_revision: m.template_revision,
        template_artifact_digest: m.template_artifact_digest,
        template_attestation_id: m.template_attestation_id,
        template_inputs: serde_json::from_str(&m.template_inputs_json).unwrap_or_default(),
        inputs_digest: m.inputs_digest,
        weight: u32::try_from(m.weight).unwrap_or_default(),
        max_runners: m.max_runners,
    }
}

impl SqliteControlPlane {
    pub(crate) async fn template_pool_revision_row(
        &self,
        key: &str,
    ) -> CoreResult<Option<TemplatePoolRevision>> {
        let Some(revision) = self
            .store
            .template_pool_revision_latest(key)
            .await
            .map_err(core_err)?
        else {
            return Ok(None);
        };
        let members = self
            .store
            .template_pool_members(key, revision.revision)
            .await
            .map_err(core_err)?
            .into_iter()
            .map(pool_member_row)
            .collect();
        Ok(Some(TemplatePoolRevision {
            pool_key: revision.pool_key,
            revision: revision.revision,
            spec_json: revision.spec_json,
            failure_policy: if revision.failure_policy == "redistribute" {
                shaula_core::template_pool::PoolFailurePolicy::Redistribute
            } else {
                shaula_core::template_pool::PoolFailurePolicy::Backpressure
            },
            members,
            actor: revision.actor,
            created_at: revision.created_at,
        }))
    }
}
