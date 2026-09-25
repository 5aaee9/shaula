use super::mapping::{
    auth_handoff_row, auth_profile_head, fleet_head, fleet_pool_member_row, fleet_revision_row,
    pool_member_row, template_row, tpl_profile_head,
};
use super::{core_err, SqliteControlPlane};

use shaula_core::error::CoreResult;
use shaula_core::registry::{
    AuthHandoffRow, FleetHead, FleetRevisionRow, ProfileHead, TemplateRevisionRow,
};

impl SqliteControlPlane {
    pub(super) async fn fleet_get_read(&self, key: &str) -> CoreResult<Option<FleetHead>> {
        Ok(self
            .store
            .fleet_get(key)
            .await
            .map_err(core_err)?
            .map(fleet_head))
    }
    pub(super) async fn fleet_list_read(
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
    pub(super) async fn fleet_revision_latest_read(
        &self,
        key: &str,
    ) -> CoreResult<Option<FleetRevisionRow>> {
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
    pub(super) async fn generation_lookup_read(
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
    pub(super) async fn handoff_get_read(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<AuthHandoffRow>> {
        self.store
            .handoff_get(fleet_key)
            .await
            .map_err(core_err)?
            .map(auth_handoff_row)
            .transpose()
            .map_err(core_err)
    }
    pub(super) async fn template_profile_get_read(
        &self,
        key: &str,
    ) -> CoreResult<Option<ProfileHead>> {
        Ok(self
            .store
            .template_profile_get(key)
            .await
            .map_err(core_err)?
            .map(tpl_profile_head))
    }
    pub(super) async fn template_profile_keys_read(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .store
            .template_profiles_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| p.key)
            .collect())
    }
    pub(super) async fn template_revision_get_read(
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
    pub(super) async fn auth_profile_get_read(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        Ok(self
            .store
            .auth_profile_get(key)
            .await
            .map_err(core_err)?
            .map(auth_profile_head))
    }
}
