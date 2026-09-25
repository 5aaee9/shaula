use super::mapping::{auth_row, fleet_change_row, profile_change_row};
use super::{core_err, SqliteControlPlane};

use shaula_core::error::CoreResult;
use shaula_core::registry::{AuthRevisionRow, ChangeView};

impl SqliteControlPlane {
    pub(super) async fn auth_profile_keys_read(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .store
            .auth_profiles_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| p.key)
            .collect())
    }
    pub(super) async fn auth_revision_get_read(
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
    pub(super) async fn auth_revision_active_read(
        &self,
        key: &str,
    ) -> CoreResult<Option<AuthRevisionRow>> {
        Ok(self
            .store
            .auth_revision_active(key)
            .await
            .map_err(core_err)?
            .map(auth_row))
    }
    pub(super) async fn fleet_change_get_read(
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
    pub(super) async fn profile_change_get_read(
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
    pub(super) async fn idempotency_find_read(
        &self,
        principal: &str,
        operation: &str,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<shaula_core::registry::IdempotencyLookup> {
        if self
            .store
            .idempotency_find_by_key(None, None, resource_kind, resource_key, idempotency_key)
            .await
            .map_err(core_err)?
            .is_some()
        {
            return Ok(shaula_core::registry::IdempotencyLookup::LegacyConflict);
        }
        let Some(record) = self
            .store
            .idempotency_find_by_key(
                Some(principal),
                Some(operation),
                resource_kind,
                resource_key,
                idempotency_key,
            )
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
    pub(super) async fn template_pool_get_read(
        &self,
        key: &str,
    ) -> CoreResult<Option<shaula_core::template_pool::TemplatePoolHead>> {
        Ok(self
            .store
            .template_pool_get(key)
            .await
            .map_err(core_err)?
            .map(|p| shaula_core::template_pool::TemplatePoolHead {
                key: p.key,
                incarnation: p.incarnation,
                desired_revision: p.desired_revision,
                phase: p.phase,
                deletion_marker: p.deletion_marker,
                tombstone: p.tombstone,
            }))
    }

    pub(super) async fn template_pool_list_read(&self) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(self
            .store
            .template_pool_list()
            .await
            .map_err(core_err)?
            .into_iter()
            .map(|p| (p.key, p.desired_revision, p.incarnation))
            .collect())
    }

    pub(super) async fn template_pool_change_get_read(
        &self,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
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
