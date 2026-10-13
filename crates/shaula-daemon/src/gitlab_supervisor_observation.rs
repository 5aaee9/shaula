//! Inventory classification and replacement status observations.

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::ports::gitlab::GitlabRunnerRef;
use shaula_core::registry::FleetObservation;

use super::{GitlabPoolSupervisor, GitlabReconcileReport};
use crate::runner_operation::gitlab::is_owned_generation;

impl GitlabPoolSupervisor {
    /// Every registration under the prefix is an anchor of this Fleet, an
    /// occupying Generation's exact registration, or unknown.
    pub(super) async fn classify_inventory(
        &self,
        runners: &[GitlabRunnerRef],
        report: &mut GitlabReconcileReport,
    ) -> CoreResult<()> {
        let mut identities = std::collections::BTreeMap::new();
        for generation in self
            .lifecycle
            .generations_for_fleet(&self.fleet_key)
            .await?
        {
            if generation.state.counts_occupancy() {
                if let Some(id) = self
                    .lifecycle
                    .generation_gitlab_runner(&generation.id)
                    .await?
                {
                    identities.insert(id, generation);
                }
            }
        }
        for runner in runners.iter().filter(|r| !self.is_anchor(r)) {
            let owned = i64::try_from(runner.id)
                .ok()
                .and_then(|id| identities.get(&id))
                .is_some_and(|generation| is_owned_generation(runner, &self.fleet_key, generation));
            if owned && runner.is_known() {
                report.owned_runners = report.owned_runners.saturating_add(1);
                report.idle_runners += u64::from(runner.is_online_idle() && !runner.paused);
                report.active_runners += u64::from(runner.is_active());
            } else {
                report.unknown_runners = report.unknown_runners.saturating_add(1);
            }
        }
        report.surplus_runners = report.owned_runners.saturating_sub(report.target);
        if report.unknown_runners > 0 {
            report.reason.get_or_insert(ReasonCode::UnknownRemoteRunner);
        }
        Ok(())
    }

    pub(super) async fn observe_status(
        &self,
        report: &GitlabReconcileReport,
        deleting: bool,
        now: i64,
    ) -> CoreResult<()> {
        let generations = self
            .lifecycle
            .generations_for_fleet(&self.fleet_key)
            .await?;
        let occupied = generations
            .iter()
            .any(|generation| generation.state.counts_occupancy());
        if deleting {
            if !occupied && !report.stale_inventory && report.unknown_runners == 0 {
                let _gate = self.gates.acquire_exclusive(&self.fleet_key).await;
                // Anchors share the prefix, so an empty inventory also proves
                // every anchor removed.
                if self
                    .current_head()
                    .await?
                    .is_some_and(|head| head.deletion_marker)
                    && self.store.generations_occupancy(&self.fleet_key).await? == 0
                    && self
                        .lifecycle
                        .gitlab_anchors(&self.fleet_key)
                        .await?
                        .is_empty()
                    && self
                        .gitlab
                        .list_runners(&self.runner_name_prefix)
                        .await
                        .is_ok_and(|runners| runners.is_empty())
                {
                    self.lifecycle
                        .fleet_set_tombstone(&self.fleet_key, now)
                        .await?;
                }
            }
            return Ok(());
        }
        let (phase, reason) = crate::pool_driver::phase(
            &generations,
            self.guard.desired_revision,
            &crate::pool_driver::PassFacts {
                stale: report.stale_demand || report.stale_inventory,
                reason: report.reason,
                target: report.target,
                serving: report.idle_runners.saturating_add(report.active_runners),
            },
        );
        self.lifecycle
            .fleet_set_observed(
                &self.fleet_key,
                &FleetObservation {
                    guard: self.guard.clone(),
                    session_epoch: None,
                    phase,
                    reason,
                },
                now,
            )
            .await?;
        Ok(())
    }
}
