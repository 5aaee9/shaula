//! Capacity accounting over the Generation ledger.

use shaula_core::error::CoreResult;

use super::FleetSupervisor;

impl FleetSupervisor {
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
