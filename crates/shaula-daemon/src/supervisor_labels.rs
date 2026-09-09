//! In-place routing configuration for a proven-owned Scale Set.

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::ports::{AccessFailure, EffectOutcome};

use super::{fingerprint, FleetSupervisor, LookupOutcome, OwnershipOutcome};

impl FleetSupervisor {
    pub(super) async fn reconcile_labels(
        &self,
        bound_id: i64,
        group_id: i64,
        now: i64,
    ) -> CoreResult<OwnershipOutcome> {
        let Some(listener) = &self.listener else {
            self.upsert_ownership(Some(bound_id), "AccessBlocked", None, now)
                .await?;
            return Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipProofFailed));
        };
        // ensure_ownership holds the shared Fleet exclusive effect gate through
        // lookup, pending writes, PUT and readback, including recovery.
        if !listener.authorize_runtime().await? {
            return Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipProofFailed));
        }
        let owned = self.store.scale_set_get(&self.config.fleet_key).await?;
        if !owned.is_some_and(|row| {
            row.scale_set_id == Some(bound_id)
                && row.owned_scale_set_id == Some(bound_id)
                && row.fingerprint == fingerprint(&self.identity)
                && row.name == self.identity.scale_set_name
                && row.runner_group == self.identity.runner_group
        }) {
            return Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipProofFailed));
        }
        // Closing the durable acquisition guard survives timeouts and restart.
        self.upsert_ownership(Some(bound_id), "LabelsUpdating", None, now)
            .await?;
        if let Err(failure) = self.github.ensure_route_proof().await {
            return self
                .access_blocked(Some(bound_id), None, &failure, now)
                .await;
        }
        let view = match self.github.lookup_scale_set(&self.identity, group_id).await {
            Ok(LookupOutcome::ExactlyOne(view))
                if view.id == bound_id && self.identity_compatible(&view, group_id) =>
            {
                view
            }
            Ok(_) => return Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipConflict)),
            Err(failure) => {
                return self
                    .access_blocked(Some(bound_id), None, &failure, now)
                    .await
            }
        };
        let inventory = self.check_inventory(bound_id, now).await?;
        if matches!(inventory, OwnershipOutcome::Blocked(_)) {
            return Ok(inventory);
        }
        if self.view_compatible(&view, group_id) {
            self.upsert_ownership(Some(bound_id), "Adopted", None, now)
                .await?;
            return Ok(OwnershipOutcome::Ready);
        }
        if !listener.authorize_runtime().await? {
            return Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipProofFailed));
        }
        match self
            .github
            .update_scale_set_labels(bound_id, &self.fallback_labels())
            .await
        {
            Ok(EffectOutcome::Definite(_)) => {}
            Ok(EffectOutcome::Uncertain { .. }) | Err(AccessFailure::RequestUncertain { .. }) => {
                self.upsert_ownership(Some(bound_id), "LabelsUpdateUncertain", None, now)
                    .await?;
                return Ok(OwnershipOutcome::Blocked(
                    ReasonCode::ScaleSetLabelsUpdateUncertain,
                ));
            }
            Err(failure) => {
                return self
                    .access_blocked(Some(bound_id), None, &failure, now)
                    .await
            }
        }
        // Never mark Ready merely because PUT returned 200.
        match self.github.lookup_scale_set(&self.identity, group_id).await {
            Ok(LookupOutcome::ExactlyOne(view))
                if view.id == bound_id && self.view_compatible(&view, group_id) =>
            {
                self.verify_inventory(bound_id, now).await
            }
            Ok(_) => Ok(OwnershipOutcome::Blocked(ReasonCode::ScaleSetLabelsPending)),
            Err(failure) => {
                self.access_blocked(Some(bound_id), None, &failure, now)
                    .await
            }
        }
    }
}
