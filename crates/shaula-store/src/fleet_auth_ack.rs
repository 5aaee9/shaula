//! Atomic handoff authority checks and observed context acknowledgement.
use crate::entities::auth::fleet_auth_contexts;
use crate::entities::fleet::fleet_auth_handoffs;
use crate::store::{Store, StoreError, StoreResult};
use sea_orm::ActiveValue::Set;
use sea_orm::{DatabaseTransaction, EntityTrait};

impl Store {
    /// Durably advances the observed tuple to the full desired tuple after
    /// successful classification. A v2 `context_json` is verified against
    /// the durable pin and written in the SAME transaction — the observed
    /// ref and the observed context can never diverge (spec 0011 §5.2
    /// step 4). A pin conflict writes nothing.
    pub(crate) async fn handoff_acknowledge(
        &self,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
        context_json: Option<&str>,
        expectation: &shaula_core::registry::AuthHandoffExpectation,
    ) -> StoreResult<shaula_core::registry::FleetContextAck> {
        let tx = self.begin().await?;
        let result = self
            .handoff_acknowledge_tx(
                &tx,
                fleet_key,
                profile_key,
                revision,
                context_json,
                expectation,
            )
            .await;
        match result {
            Ok(ack) => tx.commit().await.map(|_| ack).map_err(StoreError::from),
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    async fn handoff_acknowledge_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
        context_json: Option<&str>,
        expectation: &shaula_core::registry::AuthHandoffExpectation,
    ) -> StoreResult<shaula_core::registry::FleetContextAck> {
        use shaula_core::registry::FleetContextAck;
        // ---- CHECK PHASE: every CAS precondition is validated BEFORE any
        // write, so a refusal (Stale / IdentityDrift) leaves BOTH the
        // handoff tuple and the context row untouched (spec 0011 §5.2
        // step 4: observed ref and observed context advance together).
        let handoff = fleet_auth_handoffs::Entity::find_by_id(fleet_key.to_string())
            .one(tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("handoff {fleet_key} missing")))?;
        if handoff.desired_profile_key != profile_key || handoff.desired_revision != revision {
            // The desired tuple moved on; this acknowledgement is stale.
            return Ok(FleetContextAck::Stale);
        }

        // G5: the fence captured BEFORE network validation must still be
        // the CURRENT fence — a concurrent fleet PUT invalidates this
        // in-flight acknowledgement regardless of the stored intent.
        let current_fence =
            crate::entities::fleet::fleets::Entity::find_by_id(fleet_key.to_string())
                .one(tx)
                .await?
                .map(|f| f.mutation_fence);
        if Some(expectation.mutation_fence) != current_fence {
            return Ok(FleetContextAck::Stale);
        }

        // The revision's STORED SCHEMA decides ref-only vs exact (F7):
        // a v2 desired revision can never settle through ref equality
        // alone, with or without a context payload.
        let Some(revision_row) = self.auth_revision_get_tx(tx, profile_key, revision).await? else {
            return Ok(FleetContextAck::Stale);
        };
        let is_v2 = revision_row.schema_version >= 2;
        let Some(context_json) = context_json else {
            if is_v2 || expectation.desired_context_json.is_some() {
                return Ok(FleetContextAck::Stale);
            }
            // Legacy revision: resolution is ref-only — the ref CAS above
            // is the whole contract.
            self.handoff_write_observed(tx, fleet_key, profile_key, revision)
                .await?;
            return Ok(FleetContextAck::NotApplicable);
        };

        let Some(context_row) =
            crate::entities::auth::fleet_auth_contexts::Entity::find_by_id(fleet_key.to_string())
                .one(tx)
                .await?
        else {
            // A v2 desired revision without its context intent is a
            // broken promotion invariant, never ref-only ack material.
            return Ok(FleetContextAck::Stale);
        };
        if context_row.desired_profile_key.as_deref() != Some(profile_key)
            || context_row.desired_revision != Some(revision)
            || context_row.desired_context_json != expectation.desired_context_json
        {
            return Ok(FleetContextAck::Stale);
        }
        // Fence CAS: the fleet's CURRENT mutation fence must still equal
        // the fence captured with the desired intent — a concurrent fleet
        // PUT moved the target under this acknowledgement.
        let current_fence =
            crate::entities::fleet::fleets::Entity::find_by_id(fleet_key.to_string())
                .one(tx)
                .await?
                .map(|f| f.mutation_fence);
        match (context_row.desired_fence, current_fence) {
            (Some(captured), Some(current)) if captured != current => {
                return Ok(FleetContextAck::Stale);
            }
            (None, _) | (_, None) => return Ok(FleetContextAck::Stale),
            _ => {}
        }
        let new_context: shaula_core::auth_context::ResolvedAuthContext =
            serde_json::from_str(context_json)
                .map_err(|e| StoreError::Corrupt(format!("resolved context invalid: {e}")))?;
        // F7: the verified context must be the SAME authority the desired
        // intent declared — profile ref, target and route binding — a
        // mismatch is stale work, refused without writes.
        match &context_row.desired_context_json {
            Some(desired_json) => {
                let Ok(desired) = serde_json::from_str::<
                    shaula_core::auth_context::ResolvedAuthContext,
                >(desired_json) else {
                    return Err(StoreError::Corrupt(
                        "desired context intent unreadable".into(),
                    ));
                };
                if !new_context.completes(&desired) {
                    return Ok(FleetContextAck::IdentityDrift);
                }
            }
            None => return Ok(FleetContextAck::Stale),
        }
        // Corrupt observed evidence is fail-closed, never silently
        // skipped.
        if let Some(observed_json) = context_row.observed_context_json.as_deref() {
            let observed: shaula_core::auth_context::ResolvedAuthContext =
                serde_json::from_str(observed_json)
                    .map_err(|e| StoreError::Corrupt(format!("observed context corrupt: {e}")))?;
            if context_row.observed_profile_key.as_deref() != Some(observed.profile_key.as_str())
                || context_row.observed_revision != Some(observed.revision)
                || !observed.has_complete_identity()
            {
                return Err(StoreError::Corrupt(
                    "observed context reference or identity mismatched".into(),
                ));
            }
            if let Some((key, revision)) = handoff
                .observed_profile_key
                .as_deref()
                .zip(handoff.observed_revision)
            {
                let auth = self
                    .auth_revision_get_tx(tx, key, revision)
                    .await?
                    .ok_or_else(|| StoreError::Corrupt("observed auth revision missing".into()))?;
                // A legacy handoff retains the last v2 context for cleanup;
                // an executing v2 ref must match its exact observed evidence.
                if auth.schema_version >= 2
                    && (key != observed.profile_key || revision != observed.revision)
                {
                    return Err(StoreError::Corrupt(
                        "handoff observed reference disagrees with context".into(),
                    ));
                }
            } else {
                return Err(StoreError::Corrupt(
                    "context exists without observed handoff reference".into(),
                ));
            }
            if observed.target == new_context.target
                && !shaula_core::registry::auth_context_pins_agree(&observed, &new_context)
            {
                return Ok(FleetContextAck::IdentityDrift);
            }
        }

        // ---- WRITE PHASE: exact authority is retained for cleanup/recovery.
        self.auth_archive_context_tx(tx, fleet_key, &new_context)
            .await?;
        if let Some(json) = &context_row.observed_context_json {
            let observed = serde_json::from_str(json)
                .map_err(|e| StoreError::Corrupt(format!("observed context corrupt: {e}")))?;
            self.auth_archive_context_tx(tx, fleet_key, &observed)
                .await?;
        }
        self.handoff_write_observed(tx, fleet_key, profile_key, revision)
            .await?;
        let mut context_updated: crate::entities::auth::fleet_auth_contexts::ActiveModel =
            context_row.into();
        context_updated.observed_profile_key = Set(Some(profile_key.to_string()));
        context_updated.observed_revision = Set(Some(revision));
        context_updated.observed_context_json = Set(Some(context_json.to_string()));
        context_updated.state = Set("Observed".to_string());
        context_updated.reason = Set(None);
        context_updated.next_retry_at = Set(None);
        fleet_auth_contexts::Entity::update(context_updated)
            .exec(tx)
            .await?;
        Ok(FleetContextAck::Acknowledged)
    }

    /// The shared handoff-row write of the acknowledge phase.
    async fn handoff_write_observed(
        &self,
        tx: &DatabaseTransaction,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
    ) -> StoreResult<()> {
        let row = fleet_auth_handoffs::Entity::find_by_id(fleet_key.to_string())
            .one(tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("handoff {fleet_key} missing")))?;
        let mut updated: fleet_auth_handoffs::ActiveModel = row.into();
        updated.observed_profile_key = Set(Some(profile_key.to_string()));
        updated.observed_revision = Set(Some(revision));
        updated.state = Set("Observed".to_string());
        updated.reason = Set(None);
        updated.next_retry_at = Set(None);
        fleet_auth_handoffs::Entity::update(updated)
            .exec(tx)
            .await?;
        Ok(())
    }
}
