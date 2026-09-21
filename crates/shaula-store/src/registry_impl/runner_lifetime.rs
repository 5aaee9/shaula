//! Hard timeout checkpoints stay separate from mutable generation observations.

use async_trait::async_trait;
use sea_orm::{ActiveValue::Set, EntityTrait};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState;
use shaula_core::runner_lifetime::{RunnerLifetime, RunnerLifetimeStore};

use super::{core_err, SqliteControlPlane};
use crate::entities::lifecycle::runner_generations;

fn missing() -> CoreError {
    CoreError::new(
        ReasonCode::StorageCorrupt,
        "runner lifetime generation missing",
    )
}

#[async_trait]
impl RunnerLifetimeStore for SqliteControlPlane {
    async fn generation_lifetime(&self, id: &str) -> CoreResult<RunnerLifetime> {
        let row = self
            .store
            .generation_get(id)
            .await
            .map_err(core_err)?
            .ok_or_else(missing)?;
        Ok(RunnerLifetime {
            provisioned_at: row.provisioned_at,
            expiry_requested_at: row.expiry_requested_at,
            resources_destroyed_at: row.resources_destroyed_at,
        })
    }

    async fn generation_request_expiry(&self, id: &str, now: i64) -> CoreResult<bool> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let row = runner_generations::Entity::find_by_id(id.to_owned())
            .one(&tx)
            .await
            .map_err(crate::StoreError::from)
            .map_err(core_err)?
            .ok_or_else(missing)?;
        if row.provisioned_at.is_none()
            || matches!(
                row.state.as_str(),
                "CreatePending" | "Quarantined" | "Destroyed"
            )
        {
            return Ok(false);
        }
        if row.expiry_requested_at.is_none() {
            let mut updated: runner_generations::ActiveModel = row.into();
            updated.expiry_requested_at = Set(Some(now));
            runner_generations::Entity::update(updated)
                .exec(&tx)
                .await
                .map_err(crate::StoreError::from)
                .map_err(core_err)?;
        }
        tx.commit()
            .await
            .map_err(crate::StoreError::from)
            .map_err(core_err)?;
        Ok(true)
    }

    async fn generation_resources_destroyed(&self, id: &str, now: i64) -> CoreResult<()> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let row = runner_generations::Entity::find_by_id(id.to_owned())
            .one(&tx)
            .await
            .map_err(crate::StoreError::from)
            .map_err(core_err)?
            .ok_or_else(missing)?;
        if row.expiry_requested_at.is_none()
            || row.state != GenerationState::Destroying.as_str_repr()
        {
            return Err(CoreError::new(
                ReasonCode::OwnershipConflict,
                "resource cleanup lacks expiry intent",
            ));
        }
        if row.resources_destroyed_at.is_none() {
            let mut updated: runner_generations::ActiveModel = row.into();
            updated.resources_destroyed_at = Set(Some(now));
            runner_generations::Entity::update(updated)
                .exec(&tx)
                .await
                .map_err(crate::StoreError::from)
                .map_err(core_err)?;
        }
        tx.commit()
            .await
            .map_err(crate::StoreError::from)
            .map_err(core_err)?;
        Ok(())
    }
}
