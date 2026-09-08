//! Bounded validation evidence and current Fleet conditions are display-only.

use shaula_core::auth_context::{AccountBinding, ResolvedAuthContext, POSITIVE_PROOF_TTL_MS};
use shaula_core::registry::{
    AuthBindingHealth, AuthHandoffRow, AuthRouteObservation, FleetAuthContextRow,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn binding_health(
    key: &str,
    revision: i64,
    binding: &AccountBinding,
    routes: &[(FleetAuthContextRow, Option<AuthHandoffRow>)],
    observations: &[AuthRouteObservation],
    retained: &[(&str, &ResolvedAuthContext)],
    now: i64,
) -> AuthBindingHealth {
    let mut relevant = BTreeMap::<&str, Vec<ResolvedAuthContext>>::new();
    let mut blocked = BTreeSet::new();
    // Live generations and sessions can retain an older execution route
    // after both desired and observed Fleet heads have advanced.
    for (fleet, context) in retained {
        if context.matches_ref_and_binding(key, revision, binding) {
            relevant.entry(fleet).or_default().push((*context).clone());
        }
    }
    for (row, handoff) in routes {
        let mut desired_matches = false;
        for (reference, json, desired) in [
            (&row.desired, &row.desired_context_json, true),
            (&row.observed, &row.observed_context_json, false),
        ] {
            if reference
                .as_ref()
                .is_none_or(|(profile, rev)| profile != key || *rev != revision)
            {
                continue;
            }
            let Some(context) = json
                .as_deref()
                .and_then(|json| serde_json::from_str::<ResolvedAuthContext>(json).ok())
            else {
                continue;
            };
            if !context.matches_ref_and_binding(key, revision, binding) {
                continue;
            }
            desired_matches |= desired;
            relevant.entry(&row.fleet_key).or_default().push(context);
        }
        // A failure of the newer desired route cannot taint a retained older
        // observed revision, even when they use the same account/installation.
        let handoff_blocked = handoff.as_ref().is_some_and(|handoff| {
            handoff.desired.0 == key && handoff.desired.1 == revision && handoff.state == "Blocked"
        });
        if desired_matches && (row.state == "Blocked" || handoff_blocked) {
            blocked.insert(row.fleet_key.clone());
        }
    }
    let mut fresh = BTreeMap::<&str, &AuthRouteObservation>::new();
    for observation in observations {
        if !observation
            .context
            .matches_ref_and_binding(key, revision, binding)
            || observation.checked_at_ms > now
            || observation.valid_until_ms <= now
        {
            continue;
        }
        let matches_current =
            relevant
                .get(observation.fleet_key.as_str())
                .is_some_and(|contexts| {
                    contexts.iter().any(|context| {
                        observation.context == *context || observation.context.completes(context)
                    })
                });
        if matches_current
            && fresh
                .get(observation.fleet_key.as_str())
                .is_none_or(|previous| previous.checked_at_ms <= observation.checked_at_ms)
        {
            fresh.insert(&observation.fleet_key, observation);
        }
    }
    let failures: Vec<_> = fresh
        .values()
        .filter(|observation| !observation.healthy)
        .collect();
    blocked.extend(
        failures
            .iter()
            .map(|observation| observation.fleet_key.clone()),
    );
    if !blocked.is_empty() {
        return AuthBindingHealth {
            account_id: binding.account_id,
            installation_id: binding.installation_id,
            state: if blocked.len() == relevant.len() {
                "Blocked"
            } else {
                "Degraded"
            }
            .into(),
            reason: failures
                .first()
                .and_then(|observation| observation.reason.clone())
                .or_else(|| Some("FleetAuthBlocked".into())),
            checked_at_ms: failures
                .iter()
                .map(|observation| observation.checked_at_ms)
                .min(),
            valid_until_ms: failures
                .iter()
                .map(|observation| observation.valid_until_ms)
                .min(),
            affected_fleets: blocked.into_iter().collect(),
        };
    }
    if !relevant.is_empty() && fresh.len() == relevant.len() {
        return AuthBindingHealth {
            account_id: binding.account_id,
            installation_id: binding.installation_id,
            state: "Healthy".into(),
            reason: Some("CurrentFleetAccessVerified".into()),
            checked_at_ms: fresh
                .values()
                .map(|observation| observation.checked_at_ms)
                .min(),
            valid_until_ms: fresh
                .values()
                .map(|observation| observation.valid_until_ms)
                .min(),
            affected_fleets: Vec::new(),
        };
    }
    let expires = binding
        .validated_at_ms
        .saturating_add(POSITIVE_PROOF_TTL_MS);
    let validated = binding.validated_at_ms > 0 && binding.validated_at_ms <= now && now < expires;
    AuthBindingHealth {
        account_id: binding.account_id,
        installation_id: binding.installation_id,
        state: if validated { "Validated" } else { "Unknown" }.into(),
        reason: Some(
            if validated {
                "CandidateValidationOnly"
            } else {
                "NoFreshAccessObservation"
            }
            .into(),
        ),
        checked_at_ms: validated.then_some(binding.validated_at_ms),
        valid_until_ms: validated.then_some(expires),
        affected_fleets: Vec::new(),
    }
}

#[cfg(test)]
#[path = "service_auth_health_tests.rs"]
mod tests;
