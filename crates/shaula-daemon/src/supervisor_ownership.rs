//! Fleet scale-set ownership: create-or-adopt with durable identity, runner
//! group, label and inventory proofs. Every blocked pass has a finite reason.

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::ports::{AccessFailure, EffectOutcome, ScaleSetView};

use super::{fingerprint, FleetSupervisor, LookupOutcome, OwnershipOutcome};

impl FleetSupervisor {
    /// Re-verifies the exact persisted binding on every tick. Identity drift
    /// never silently rebinds a fleet with live resources (R9-05/R10-03).
    async fn verify_adopted(&self, bound_id: i64, now: i64) -> CoreResult<OwnershipOutcome> {
        let row = self.store.scale_set_get(&self.config.fleet_key).await?;
        let attempt = row.as_ref().and_then(|r| r.attempt_id.clone());
        if row
            .as_ref()
            .is_some_and(|r| r.fingerprint != fingerprint(&self.identity))
        {
            return self
                .reopen_ownership(ReasonCode::OwnershipProofFailed, attempt, now)
                .await;
        }
        let group_id = match self.github.resolve_runner_group(&self.identity).await {
            Ok(id) => id,
            Err(failure) => {
                return self
                    .access_blocked(Some(bound_id), attempt, &failure, now)
                    .await;
            }
        };
        match self.github.lookup_scale_set(&self.identity, group_id).await {
            Ok(LookupOutcome::ExactlyOne(view))
                if view.id == bound_id && self.view_compatible(&view, group_id) =>
            {
                self.verify_inventory(bound_id, now).await
            }
            Ok(LookupOutcome::ExactlyOne(view)) if view.id == bound_id => {
                // The bound identity still exists, but its routing changed.
                self.upsert_ownership(Some(bound_id), "AccessBlocked", attempt, now)
                    .await?;
                Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipConflict))
            }
            Ok(LookupOutcome::ExactlyOne(_)) | Ok(LookupOutcome::None) => {
                self.reopen_ownership(ReasonCode::ScaleSetMissing, attempt, now)
                    .await
            }
            Ok(LookupOutcome::Multiple) => {
                self.reopen_ownership(ReasonCode::OwnershipConflict, attempt, now)
                    .await
            }
            Err(failure) => {
                self.access_blocked(Some(bound_id), attempt, &failure, now)
                    .await
            }
        }
    }

    /// Reclassification is allowed only without retained resource occupancy.
    async fn reopen_ownership(
        &self,
        empty_reason: ReasonCode,
        attempt: Option<String>,
        now: i64,
    ) -> CoreResult<OwnershipOutcome> {
        let (_, occupancy) = self.capacity_counters().await?;
        let (state, reason) = if occupancy > 0 {
            (
                "ScaleSetMissingWithResources",
                ReasonCode::ScaleSetMissingWithResources,
            )
        } else {
            ("Unbound", empty_reason)
        };
        self.upsert_ownership(None, state, attempt, now).await?;
        Ok(OwnershipOutcome::Blocked(reason))
    }

    async fn access_blocked(
        &self,
        scale_set_id: Option<i64>,
        attempt: Option<String>,
        failure: &AccessFailure,
        now: i64,
    ) -> CoreResult<OwnershipOutcome> {
        self.upsert_ownership(scale_set_id, "AccessBlocked", attempt, now)
            .await?;
        Ok(OwnershipOutcome::access_failure(failure))
    }

    /// Every required label must be observable. The adapter normalizes known
    /// wire label types; names and distinct types still match exactly.
    fn labels_compatible(&self, observed: &[shaula_core::github::Label]) -> bool {
        self.fallback_labels().iter().all(|required| {
            observed
                .iter()
                .any(|label| label.name == required.name && label.label_type == required.label_type)
        })
    }

    fn view_compatible(&self, view: &ScaleSetView, group_id: i64) -> bool {
        view.id > 0
            && view.name == self.identity.scale_set_name
            && view.runner_group_id == group_id
            && self.labels_compatible(&view.labels)
    }

    /// Persist-before-POST create-or-adopt (spec 0001 section 7). Adoption proves
    /// identity and routing; a same-name match alone never grants ownership.
    pub(crate) async fn ensure_ownership(&self, now: i64) -> CoreResult<OwnershipOutcome> {
        let proof = self.github.ensure_route_proof().await;
        self.record_route_health(&proof, now).await?;
        if let Err(failure) = proof {
            return Ok(OwnershipOutcome::access_failure(&failure));
        }
        let existing = self.store.scale_set_get(&self.config.fleet_key).await?;
        if let Some(row) = &existing {
            if matches!(
                row.state.as_str(),
                "Adopted" | "AccessBlocked" | "UnknownRemoteRunner"
            ) {
                if let Some(bound_id) = row.scale_set_id {
                    return self.verify_adopted(bound_id, now).await;
                }
            }
            if row.state == "ScaleSetMissingWithResources" {
                return Ok(OwnershipOutcome::Blocked(
                    ReasonCode::ScaleSetMissingWithResources,
                ));
            }
            // An access failure blocks re-POST until its exact credential
            // handoff recovers; never fall back to another auth revision.
            if row.state == "AccessBlocked" {
                let handoff_ok = self
                    .handoff
                    .handoff_get(&self.config.fleet_key)
                    .await?
                    .is_some_and(|h| h.observed == Some(h.desired));
                if !handoff_ok {
                    return Ok(OwnershipOutcome::Blocked(
                        ReasonCode::AccessVerificationFailed,
                    ));
                }
                self.upsert_ownership(row.scale_set_id, "Unbound", row.attempt_id.clone(), now)
                    .await?;
            }
        }

        let attempt = existing.as_ref().and_then(|row| row.attempt_id.clone());
        let group_id = match self.github.resolve_runner_group(&self.identity).await {
            Ok(id) => id,
            Err(failure) => return self.access_blocked(None, attempt, &failure, now).await,
        };
        match self.github.lookup_scale_set(&self.identity, group_id).await {
            Ok(LookupOutcome::ExactlyOne(view)) => {
                if !self.view_compatible(&view, group_id) {
                    self.upsert_ownership(
                        (view.id > 0).then_some(view.id),
                        "AccessBlocked",
                        attempt,
                        now,
                    )
                    .await?;
                    return Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipConflict));
                }
                self.verify_inventory(view.id, now).await
            }
            Ok(LookupOutcome::Multiple) => {
                Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipConflict))
            }
            Ok(LookupOutcome::None) => self.create_owned_scale_set(group_id, attempt, now).await,
            Err(failure) => self.access_blocked(None, attempt, &failure, now).await,
        }
    }

    async fn create_owned_scale_set(
        &self,
        group_id: i64,
        previous_attempt: Option<String>,
        now: i64,
    ) -> CoreResult<OwnershipOutcome> {
        let attempt_id = previous_attempt.unwrap_or_else(shaula_core::auth::new_attempt_id);
        self.upsert_ownership(
            None,
            "ScaleSetCreateStarting",
            Some(attempt_id.clone()),
            now,
        )
        .await?;
        match self
            .github
            .create_scale_set(&self.identity, group_id, &self.fallback_labels())
            .await
        {
            Ok(EffectOutcome::Definite(view)) => {
                if !self.view_compatible(&view, group_id) {
                    self.upsert_ownership(
                        (view.id > 0).then_some(view.id),
                        "AccessBlocked",
                        Some(attempt_id),
                        now,
                    )
                    .await?;
                    return Ok(OwnershipOutcome::Blocked(ReasonCode::OwnershipConflict));
                }
                self.upsert_ownership(Some(view.id), "Adopted", Some(attempt_id), now)
                    .await?;
                Ok(OwnershipOutcome::Ready)
            }
            // Uncertain requests may have reached GitHub. The next tick's
            // authoritative lookup classifies the persisted attempt.
            Ok(EffectOutcome::Uncertain { .. }) | Err(AccessFailure::RequestUncertain { .. }) => {
                Ok(OwnershipOutcome::Blocked(
                    ReasonCode::ScaleSetCreateUncertain,
                ))
            }
            Err(failure) => {
                self.access_blocked(None, Some(attempt_id), &failure, now)
                    .await
            }
        }
    }

    async fn verify_inventory(&self, scale_set_id: i64, now: i64) -> CoreResult<OwnershipOutcome> {
        let runners = match self.github.list_runners(scale_set_id).await {
            Ok(runners) => runners,
            Err(failure) => {
                return self
                    .access_blocked(Some(scale_set_id), None, &failure, now)
                    .await
            }
        };
        let generations = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?;
        if runners.iter().any(|runner| {
            runner.scale_set_id != scale_set_id
                || !generations.iter().any(|generation| {
                    !generation.state.is_terminal()
                        && generation.github_runner_id == Some(runner.id)
                        && generation.runner_name == runner.name
                })
        }) {
            self.upsert_ownership(Some(scale_set_id), "UnknownRemoteRunner", None, now)
                .await?;
            return Ok(OwnershipOutcome::Blocked(ReasonCode::UnknownRemoteRunner));
        }
        self.upsert_ownership(Some(scale_set_id), "Adopted", None, now)
            .await?;
        Ok(OwnershipOutcome::Ready)
    }
}
