//! Exponential backoff between failed Runner Resource destroys of one
//! Generation. A refused destroy plan can persist for many minutes while the
//! provider cannot observe a resource; retrying every tick re-plans against
//! the provider and writes a new operation log each time.
//!
//! The failure count is process memory: after a restart the first retry is
//! immediate and the backoff starts over, which only restores the old pacing.

use std::collections::HashMap;
use std::sync::Mutex;

/// Delay after the first consecutive failure; doubled per further failure.
const BASE_MS: i64 = 15_000;
/// Upper bound, so a long-failing destroy is still retried regularly.
const CAP_MS: i64 = 300_000;

#[derive(Debug, Default)]
pub(crate) struct DestroyBackoff {
    /// Generation id → (consecutive failures, earliest next attempt).
    failures: Mutex<HashMap<String, (u32, i64)>>,
}

impl DestroyBackoff {
    /// Whether a destroy of `generation` may be attempted at `now`.
    pub(crate) fn due(&self, generation: &str, now: i64) -> bool {
        self.failures.lock().map_or(true, |map| {
            map.get(generation).is_none_or(|(_, at)| now >= *at)
        })
    }

    /// Records a failed destroy completed at `at` and returns the delay.
    pub(crate) fn failed(&self, generation: &str, at: i64) -> i64 {
        let Ok(mut map) = self.failures.lock() else {
            return 0;
        };
        let entry = map.entry(generation.to_string()).or_insert((0, at));
        entry.0 = entry.0.saturating_add(1);
        let delay = BASE_MS
            .checked_shl(entry.0 - 1)
            .filter(|delay| *delay > 0)
            .map_or(CAP_MS, |delay| delay.min(CAP_MS));
        entry.1 = at.saturating_add(delay);
        delay
    }

    /// Forgets a Generation whose destroy succeeded or ended in Quarantine.
    pub(crate) fn clear(&self, generation: &str) {
        if let Ok(mut map) = self.failures.lock() {
            map.remove(generation);
        }
    }
}
