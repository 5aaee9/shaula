//! ControlPlaneStore trait methods for shared TemplatePool resources
//! (spec 0037), split out to keep `control_plane_store` within the
//! 400-line module limit.

use super::mapping::{pool_failure_policy, pool_member_row};
use super::{core_err, SqliteControlPlane};
use shaula_core::error::CoreResult;
use shaula_core::template_pool::TemplatePoolRevision;

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
            .collect::<Result<Vec<_>, _>>()
            .map_err(core_err)?;
        Ok(Some(TemplatePoolRevision {
            pool_key: revision.pool_key,
            revision: revision.revision,
            spec_json: revision.spec_json,
            failure_policy: pool_failure_policy(&revision.failure_policy).map_err(core_err)?,
            members,
            actor: revision.actor,
            created_at: revision.created_at,
        }))
    }
}
