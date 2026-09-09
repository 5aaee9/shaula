//! Fenced runtime status and Change progression in one transaction.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use shaula_core::registry::{FleetObservation, FleetObservationPhase};

use crate::entities::{
    fleet::{fleet_changes, fleets},
    lifecycle::fleet_sessions,
};
use crate::runtime_guards::session_active;
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    pub(crate) async fn fleet_set_observed(
        &self,
        key: &str,
        observation: &FleetObservation,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.begin().await?;
        if !self
            .fleet_runtime_guard_tx(&tx, key, &observation.guard)
            .await?
        {
            return Ok(false);
        }
        let session = fleet_sessions::Entity::find_by_id(key).one(&tx).await?;
        let active_epoch = session
            .as_ref()
            .filter(|s| session_active(s))
            .map(|s| s.epoch);
        if active_epoch != observation.session_epoch {
            return Ok(false);
        }
        if observation.phase == FleetObservationPhase::Ready {
            let Some(epoch) = active_epoch else {
                return Ok(false);
            };
            let Some(context) = self.session_auth_context_tx(&tx, key).await? else {
                return Ok(false);
            };
            if !self
                .session_guard_tx(&tx, key, &observation.guard, &context, epoch)
                .await?
            {
                return Ok(false);
            }
        }
        let current = fleets::Entity::find_by_id(key)
            .one(&tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt("fleet observation head missing".into()))?;
        let mut updated: fleets::ActiveModel = current.into();
        updated.observed_revision = Set(observation.guard.desired_revision);
        updated.phase = Set(observation.phase.as_str().to_string());
        updated.last_condition_reason = Set(observation.reason.map(|r| r.as_str().to_string()));
        updated.updated_at = Set(now);
        fleets::Entity::update(updated).exec(&tx).await?;

        let changes = fleet_changes::Entity::find()
            .filter(fleet_changes::Column::FleetKey.eq(key))
            .filter(fleet_changes::Column::Revision.eq(observation.guard.desired_revision))
            .filter(fleet_changes::Column::State.is_in(["Pending", "Running", "Blocked"]))
            .all(&tx)
            .await?;
        for change in changes {
            let mut updated: fleet_changes::ActiveModel = change.into();
            updated.state = Set(match observation.phase {
                FleetObservationPhase::Reconciling => "Running",
                FleetObservationPhase::Ready => "Succeeded",
                FleetObservationPhase::Degraded => "Blocked",
            }
            .to_string());
            updated.reason = Set(observation.reason.map(|r| r.as_str().to_string()));
            updated.next_retry_at = Set(None);
            updated.updated_at = Set(now);
            fleet_changes::Entity::update(updated).exec(&tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }
}
