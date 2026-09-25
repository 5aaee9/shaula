//! Admission read projections for the SQLite adapter. Physical split only:
//! the ControlPlaneStore interface and its ownership are unchanged.

use shaula_core::error::CoreResult;
use shaula_core::registry::{
    AuthRevisionRow, ChangeView, FleetHead, FleetRevisionRow, IdempotencyLookup, ProfileHead,
    TemplateRevisionRow,
};
use shaula_core::template_pool::TemplatePoolHead;

use super::mapping::{
    auth_profile_head, auth_row, fleet_change_row, fleet_head, fleet_pool_member_row,
    fleet_revision_row, pool_member_row, profile_change_row, template_row, tpl_profile_head,
};
use super::{core_err, SqliteControlPlane};

impl SqliteControlPlane {
    pub(super) async fn template_revision_read(
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

    pub(super) async fn auth_revision_read(
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

    pub(super) async fn auth_active_read(&self, key: &str) -> CoreResult<Option<AuthRevisionRow>> {
        Ok(self
            .store
            .auth_revision_active(key)
            .await
            .map_err(core_err)?
            .map(auth_row))
    }

    pub(super) async fn fleet_change_read(
        &self,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
        Ok(self
            .store
            .change_get(change_id)
            .await
            .map_err(core_err)?
            .map(fleet_change_row))
    }

    pub(super) async fn profile_change_read(
        &self,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
        Ok(self
            .store
            .profile_change_get(change_id)
            .await
            .map_err(core_err)?
            .map(profile_change_row))
    }

    pub(super) async fn fleet_head_read(&self, key: &str) -> CoreResult<Option<FleetHead>> {
        Ok(self
            .store
            .fleet_get(key)
            .await
            .map_err(core_err)?
            .map(fleet_head))
    }

    pub(super) async fn fleet_list_read(&self) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(self
            .store
            .fleet_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|f| (f.key, f.desired_revision, f.phase))
            .collect())
    }

    pub(super) async fn fleet_revision_read(
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

    pub(super) async fn template_head_read(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        Ok(self
            .store
            .template_profile_get(key)
            .await
            .map_err(core_err)?
            .map(tpl_profile_head))
    }

    pub(super) async fn template_keys_read(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .store
            .template_profiles_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| p.key)
            .collect())
    }

    pub(super) async fn auth_head_read(&self, key: &str) -> CoreResult<Option<ProfileHead>> {
        Ok(self
            .store
            .auth_profile_get(key)
            .await
            .map_err(core_err)?
            .map(auth_profile_head))
    }

    pub(super) async fn auth_keys_read(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .store
            .auth_profiles_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| p.key)
            .collect())
    }

    pub(super) async fn idempotency_read(
        &self,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<IdempotencyLookup> {
        let Some(record) = self
            .store
            .idempotency_find_by_key(resource_kind, resource_key, idempotency_key)
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

    pub(super) async fn pool_head_read(&self, key: &str) -> CoreResult<Option<TemplatePoolHead>> {
        Ok(self
            .store
            .template_pool_get(key)
            .await
            .map_err(core_err)?
            .map(|p| TemplatePoolHead {
                key: p.key,
                incarnation: p.incarnation,
                desired_revision: p.desired_revision,
                phase: p.phase,
                deletion_marker: p.deletion_marker,
                tombstone: p.tombstone,
            }))
    }

    pub(super) async fn pool_list_read(&self) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(self
            .store
            .template_pool_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| (p.key, p.desired_revision, p.incarnation))
            .collect())
    }

    pub(super) async fn pool_change_read(&self, change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(self
            .store
            .profile_change_get(change_id)
            .await
            .map_err(core_err)?
            .filter(|c| c.resource_kind == "template_pool")
            .map(|c| ChangeView {
                id: c.id,
                resource_kind: "template_pool".to_string(),
                resource_key: c.profile_key,
                revision: c.revision.unwrap_or_default(),
                kind: c.kind,
                state: c.state,
                reason: c.reason,
            }))
    }
}
