//! Cleanup quarantine and capacity accounting.

use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState;

use super::FleetSupervisor;

impl FleetSupervisor {
    /// Quarantine unprovable cleanup after one tick, retaining occupancy.
    pub(super) async fn quarantine_stale_cleanup(&self, now: i64) -> CoreResult<u32> {
        let mut quarantined = 0;
        for generation in self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?
        {
            if generation.state != GenerationState::CleanupRequired
                || now.saturating_sub(generation.updated_at) < 60_000
                || self
                    .store
                    .generation_lifetime(&generation.id)
                    .await?
                    .expiry_requested_at
                    .is_some()
            {
                continue;
            }
            self.store
                .generation_advance(&generation.id, GenerationState::Quarantined, now)
                .await?;
            quarantined += 1;
        }
        Ok(quarantined)
    }

    pub(super) async fn capacity_counters(&self) -> CoreResult<(i64, i64)> {
        let generations = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?;
        let mut effective = 0;
        let mut occupancy = 0;
        for generation in &generations {
            effective += i64::from(generation.state.counts_effective());
            occupancy += i64::from(generation.state.counts_occupancy());
        }
        Ok((effective, occupancy))
    }
}
