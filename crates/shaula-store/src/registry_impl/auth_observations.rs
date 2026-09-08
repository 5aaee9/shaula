//! A bounded, process-local display cache. Never consulted for authorization.

use shaula_core::auth_context::{NEGATIVE_PROOF_TTL_MS, POSITIVE_PROOF_TTL_MS};
use shaula_core::registry::AuthRouteObservation;
use std::{collections::BTreeMap, sync::Mutex};

const MAX_OBSERVATIONS: usize = 10_000;
type ObservationKey = (String, String, i64);

#[derive(Default)]
pub(super) struct RouteObservations {
    entries: Mutex<BTreeMap<ObservationKey, AuthRouteObservation>>,
}

impl RouteObservations {
    pub(super) fn report(&self, mut observation: AuthRouteObservation) {
        let ttl = if observation.healthy {
            POSITIVE_PROOF_TTL_MS
        } else {
            NEGATIVE_PROOF_TTL_MS
        };
        observation.valid_until_ms = observation
            .valid_until_ms
            .min(observation.checked_at_ms.saturating_add(ttl));
        if observation.valid_until_ms <= observation.checked_at_ms
            || observation
                .reason
                .as_ref()
                .is_some_and(|reason| reason.len() > 128)
        {
            return;
        }
        let key = (
            observation.fleet_key.clone(),
            observation.context.profile_key.clone(),
            observation.context.revision,
        );
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Older completions cannot overwrite a more recent check of this route.
        if entries
            .get(&key)
            .is_some_and(|previous| previous.checked_at_ms > observation.checked_at_ms)
        {
            return;
        }
        entries.retain(|_, entry| entry.valid_until_ms > observation.checked_at_ms);
        if entries.len() >= MAX_OBSERVATIONS && !entries.contains_key(&key) {
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.checked_at_ms)
                .map(|(key, _)| key.clone())
            {
                entries.remove(&oldest);
            }
        }
        entries.insert(key, observation);
    }

    pub(super) fn read(&self, key: &str, revision: i64, now: i64) -> Vec<AuthRouteObservation> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, entry| entry.valid_until_ms > now);
        entries
            .values()
            .filter(|entry| {
                entry.context.profile_key == key
                    && entry.context.revision == revision
                    && entry.checked_at_ms <= now
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
#[path = "auth_observations_tests.rs"]
mod tests;
