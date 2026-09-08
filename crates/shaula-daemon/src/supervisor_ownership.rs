//! Fleet scale-set ownership: create-or-adopt with the R9-05/R10-03
//! proof requirements (durable fingerprint + id + runner group + label
//! compatibility) and per-tick re-verification of an existing binding.
//! Split to keep supervisor.rs within the 400-line limit (AGENTS.md).

use shaula_core::error::CoreResult;
use shaula_core::ports::EffectOutcome;

use super::{fingerprint, FleetSupervisor, LookupOutcome};

impl FleetSupervisor {
    /// Re-verifies an existing `Adopted` binding against the
    /// authoritative lookup (R9-05, R10-03; spec 0001 §7): the persisted
    /// identity FINGERPRINT must still match this supervisor's identity,
    /// the scale set must resolve to exactly-one with the SAME durable id
    /// AND runner group AND a compatible label set. Anything else
    /// re-opens classification — with live resources the fleet is held
    /// in `ScaleSetMissingWithResources` instead of blind re-creating.
    async fn verify_adopted(&self, bound_id: i64, now: i64) -> CoreResult<bool> {
        let row = self.store.scale_set_get(&self.config.fleet_key).await?;
        let attempt = row.as_ref().and_then(|r| r.attempt_id.clone());
        // R10-03: the durable fingerprint pins WHICH identity the binding
        // belongs to; a drifted fingerprint means the binding was made
        // under a different fleet identity and must be re-proven.
        let fingerprint_stale = row
            .as_ref()
            .is_some_and(|r| r.fingerprint != fingerprint(&self.identity));
        if fingerprint_stale {
            let (_, occupancy) = self.capacity_counters().await?;
            if occupancy > 0 {
                self.upsert_ownership(None, "ScaleSetMissingWithResources", attempt, now)
                    .await?;
            } else {
                self.upsert_ownership(None, "Unbound", attempt, now).await?;
            }
            return Ok(false);
        }
        let group_id = match self.github.resolve_runner_group(&self.identity).await {
            Ok(id) => id,
            Err(_) => {
                self.upsert_ownership(Some(bound_id), "AccessBlocked", attempt, now)
                    .await?;
                return Ok(false);
            }
        };
        match self.github.lookup_scale_set(&self.identity, group_id).await {
            Ok(LookupOutcome::ExactlyOne(view))
                if view.id == bound_id
                    && view.runner_group_id == group_id
                    && self.labels_compatible(&view.labels) =>
            {
                self.verify_inventory(bound_id, now).await
            }
            Ok(LookupOutcome::ExactlyOne(view)) => {
                // Exact id but drifted group/labels: stop routing until a
                // human resolves the conflict. A DIFFERENT id means our
                // bound set is gone or ambiguous: never silently re-bind
                // over live resources.
                if view.id == bound_id {
                    self.upsert_ownership(Some(bound_id), "AccessBlocked", attempt, now)
                        .await?;
                } else {
                    let (_, occupancy) = self.capacity_counters().await?;
                    if occupancy > 0 {
                        self.upsert_ownership(None, "ScaleSetMissingWithResources", attempt, now)
                            .await?;
                    } else {
                        self.upsert_ownership(None, "Unbound", attempt, now).await?;
                    }
                }
                Ok(false)
            }
            Ok(LookupOutcome::Multiple) => {
                let (_, occupancy) = self.capacity_counters().await?;
                if occupancy > 0 {
                    self.upsert_ownership(None, "ScaleSetMissingWithResources", attempt, now)
                        .await?;
                } else {
                    self.upsert_ownership(None, "Unbound", attempt, now).await?;
                }
                Ok(false)
            }
            Ok(LookupOutcome::None) => {
                let (_, occupancy) = self.capacity_counters().await?;
                if occupancy > 0 {
                    self.upsert_ownership(None, "ScaleSetMissingWithResources", attempt, now)
                        .await?;
                    Ok(false)
                } else {
                    // Genuinely gone with zero resources: clean re-adoption.
                    self.upsert_ownership(None, "Unbound", attempt, now).await?;
                    Ok(false)
                }
            }
            Err(_) => {
                self.upsert_ownership(Some(bound_id), "AccessBlocked", attempt, now)
                    .await?;
                Ok(false)
            }
        }
    }

    /// Label-compatibility proof (R9-05): every label this fleet would
    /// mint must already be observable on the scale set — otherwise
    /// jobs routed by those labels can never reach our runners.
    fn labels_compatible(&self, observed: &[shaula_core::github::Label]) -> bool {
        self.fallback_labels().iter().all(|required| {
            observed
                .iter()
                .any(|l| l.name == required.name && l.label_type == required.label_type)
        })
    }

    /// Persist-before-POST create-or-adopt (spec 0001 §7). Adoption is a
    /// PROOF, not a name match (R9-05): the observed scale set must carry
    /// a compatible label set before it may be bound, and an already
    /// `Adopted` binding is re-verified against the authoritative lookup
    /// every tick — a moved or vanished id re-opens classification
    /// instead of silently succeeding.
    pub(crate) async fn ensure_ownership(&self, now: i64) -> CoreResult<bool> {
        // Route-proof gate (spec 0011 §5.3): create-or-adopt is a new
        // management effect; it waits for fresh authorization evidence and
        // fails closed while the route is unprovable.
        let proof = self.github.ensure_route_proof().await;
        self.record_route_health(&proof, now).await?;
        if let Err(failure) = proof {
            tracing::warn!(
                fleet = %self.config.fleet_key,
                summary = %failure.summary(),
                "route proof unavailable; ownership effects blocked"
            );
            return Ok(false);
        }
        let existing = self.store.scale_set_get(&self.config.fleet_key).await?;
        if let Some(row) = &existing {
            if matches!(
                row.state.as_str(),
                "Adopted" | "AccessBlocked" | "UnknownRemoteRunner"
            ) && row.scale_set_id.is_some()
            {
                match row.scale_set_id {
                    None => {
                        // Corrupt ownership row: re-classify from scratch.
                        self.upsert_ownership(None, "Unbound", row.attempt_id.clone(), now)
                            .await?;
                    }
                    Some(bound_id) => {
                        return self.verify_adopted(bound_id, now).await;
                    }
                }
            }
            if row.state == "ScaleSetMissingWithResources" {
                return Ok(false);
            }
            // A prior access failure blocks re-POST until the credential
            // context recovers (handoff observed == desired). Never fall
            // back to a different auth kind or revision.
            if row.state == "AccessBlocked" {
                let handoff_ok = self
                    .handoff
                    .handoff_get(&self.config.fleet_key)
                    .await?
                    .map(|h| h.observed == Some(h.desired.clone()))
                    .unwrap_or(false);
                if !handoff_ok {
                    return Ok(false);
                }
                self.upsert_ownership(row.scale_set_id, "Unbound", row.attempt_id.clone(), now)
                    .await?;
            }
        }

        let group_id = match self.github.resolve_runner_group(&self.identity).await {
            Ok(id) => id,
            Err(_failure) => {
                // Access failures are conditions, not absence proofs; the
                // ownership record keeps them visible for the next tick.
                self.upsert_ownership(
                    None,
                    "AccessBlocked",
                    existing.as_ref().and_then(|r| r.attempt_id.clone()),
                    now,
                )
                .await?;
                return Ok(false);
            }
        };
        match self.github.lookup_scale_set(&self.identity, group_id).await {
            Ok(LookupOutcome::ExactlyOne(view)) => {
                // R9-05: adopt ONLY a compatible set. A same-name set with
                // a foreign label configuration would run OUR runners
                // under ITS routing — that conflict needs a human, so the
                // fleet parks in AccessBlocked instead.
                if view.id <= 0
                    || view.runner_group_id != group_id
                    || !self.labels_compatible(&view.labels)
                {
                    self.upsert_ownership(
                        Some(view.id),
                        "AccessBlocked",
                        existing.as_ref().and_then(|r| r.attempt_id.clone()),
                        now,
                    )
                    .await?;
                    return Ok(false);
                }
                if !self.verify_inventory(view.id, now).await? {
                    return Ok(false);
                }
                self.upsert_ownership(
                    Some(view.id),
                    "Adopted",
                    existing.as_ref().and_then(|r| r.attempt_id.clone()),
                    now,
                )
                .await?;
                Ok(true)
            }
            Ok(LookupOutcome::Multiple) => Ok(false),
            Ok(LookupOutcome::None) => {
                // Persist the attempt identity before the POST; retries
                // never change the name.
                let attempt_id = existing
                    .as_ref()
                    .and_then(|r| r.attempt_id.clone())
                    .unwrap_or_else(shaula_core::auth::new_attempt_id);
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
                        self.upsert_ownership(Some(view.id), "Adopted", Some(attempt_id), now)
                            .await?;
                        Ok(true)
                    }
                    // Uncertain and transport-level failures may have
                    // reached the server: never blind-retry. The next
                    // tick's authoritative lookup classifies.
                    Ok(EffectOutcome::Uncertain { .. }) => Ok(false),
                    Err(shaula_core::ports::AccessFailure::RequestUncertain { .. }) => Ok(false),
                    Err(_) => {
                        // Definite service rejection: keep the attempt but
                        // block re-POST until the credential recovers.
                        self.upsert_ownership(None, "AccessBlocked", Some(attempt_id), now)
                            .await?;
                        Ok(false)
                    }
                }
            }
            Err(failure) => {
                self.upsert_ownership(
                    None,
                    "AccessBlocked",
                    existing.as_ref().and_then(|r| r.attempt_id.clone()),
                    now,
                )
                .await?;
                let _ = failure;
                Ok(false)
            }
        }
    }

    async fn verify_inventory(&self, scale_set_id: i64, now: i64) -> CoreResult<bool> {
        let runners = match self.github.list_runners(scale_set_id).await {
            Ok(runners) => runners,
            Err(_) => {
                self.upsert_ownership(Some(scale_set_id), "AccessBlocked", None, now)
                    .await?;
                return Ok(false);
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
            return Ok(false);
        }
        self.upsert_ownership(Some(scale_set_id), "Adopted", None, now)
            .await?;
        Ok(true)
    }
}
