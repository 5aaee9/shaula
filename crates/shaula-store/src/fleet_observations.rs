//! Fenced runtime status and Change progression in one transaction.

use sea_orm::sea_query::Expr;
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use shaula_core::registry::{FleetObservation, FleetObservationPhase};

use crate::entities::{
    fleet::{fleet_changes, fleet_revisions, fleets},
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
        let head = fleets::Entity::find_by_id(key)
            .one(&tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt("fleet observation head missing".into()))?;
        // The runtime-guard CAS is shared by both branches: incarnation,
        // desired revision and mutation fence must match the captured
        // head, and a tombstoned fleet accepts no further observations.
        let guard_ok = head.incarnation == observation.guard.incarnation
            && head.desired_revision == observation.guard.desired_revision
            && head.mutation_fence == observation.guard.mutation_fence
            && !head.tombstone;
        if !guard_ok {
            return Ok(false);
        }
        if head.deletion_marker {
            // The observation is only meaningful when the deletion marker
            // came from a real DELETE: the decommission commit advances
            // the desired head AND records a Decommission change for that
            // revision in one transaction. A marker set without the change
            // (corrupt/partial state, or a stale pre-DELETE observation)
            // must not land runtime progress on the head.
            let decommission_change = fleet_changes::Entity::find()
                .filter(fleet_changes::Column::FleetKey.eq(key))
                .filter(fleet_changes::Column::Revision.eq(observation.guard.desired_revision))
                .filter(fleet_changes::Column::Kind.eq("Decommission"))
                .one(&tx)
                .await?;
            if decommission_change.is_none() {
                return Ok(false);
            }
            self.fleet_set_observed_decommissioning(&tx, key, head, observation, now)
                .await?;
            tx.commit().await?;
            return Ok(true);
        }
        let session = fleet_sessions::Entity::find_by_id(key).one(&tx).await?;
        let active_epoch = session
            .as_ref()
            .filter(|s| session_active(s))
            .map(|s| s.epoch);
        if active_epoch != observation.session_epoch {
            return Ok(false);
        }
        let forgejo = fleet_revisions::Entity::find()
            .filter(fleet_revisions::Column::FleetKey.eq(key))
            .filter(fleet_revisions::Column::Revision.eq(observation.guard.desired_revision))
            .one(&tx)
            .await?
            .and_then(|row| {
                serde_json::from_str::<shaula_core::fleet::FleetSpec>(&row.spec_json).ok()
            })
            .is_some_and(|spec| spec.kind == shaula_core::fleet::FleetProviderKind::Forgejo);
        if forgejo && active_epoch.is_some() {
            return Ok(false);
        }
        if observation.phase == FleetObservationPhase::Ready && !forgejo {
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
        let mut updated: fleets::ActiveModel = head.into();
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

    /// Decommissioning observations (spec 0002 §8): the fleet keeps its
    /// `Decommissioning` phase — a late observation must never rewrite it
    /// to a normal running phase. The runtime's progress lands on the
    /// Change and the observed head each tick; a `Ready` observation from
    /// the cleanup reconcile (every owned Generation terminal) writes the
    /// durable tombstone, `Decommissioned` phase and the Change's
    /// `Succeeded` state in this one transaction.
    async fn fleet_set_observed_decommissioning(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        key: &str,
        head: fleets::Model,
        observation: &FleetObservation,
        now: i64,
    ) -> StoreResult<()> {
        let complete = observation.phase == FleetObservationPhase::Ready;
        let mut updated: fleets::ActiveModel = head.into();
        updated.observed_revision = Set(observation.guard.desired_revision);
        if complete {
            updated.phase = Set("Decommissioned".to_string());
            updated.tombstone = Set(true);
        }
        updated.last_condition_reason = Set(observation.reason.map(|r| r.as_str().to_string()));
        updated.updated_at = Set(now);
        fleets::Entity::update(updated).exec(tx).await?;

        let changes = fleet_changes::Entity::find()
            .filter(fleet_changes::Column::FleetKey.eq(key))
            .filter(
                Expr::col(fleet_changes::Column::Revision).lte(observation.guard.desired_revision),
            )
            .filter(fleet_changes::Column::State.is_in(["Pending", "Running", "Blocked"]))
            .all(tx)
            .await?;
        for change in changes {
            let state = match observation.phase {
                FleetObservationPhase::Ready => "Succeeded",
                FleetObservationPhase::Degraded => "Blocked",
                FleetObservationPhase::Reconciling => "Running",
            };
            let mut updated: fleet_changes::ActiveModel = change.into();
            updated.state = Set(state.to_string());
            updated.reason = Set(observation.reason.map(|r| r.as_str().to_string()));
            updated.next_retry_at = Set(None);
            updated.updated_at = Set(now);
            fleet_changes::Entity::update(updated).exec(tx).await?;
        }
        Ok(())
    }
}
