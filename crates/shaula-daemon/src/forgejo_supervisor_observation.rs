//! Replacement observations: failed reads never acquire fresh timestamps.

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::ports::forgejo::{ForgejoDemandSnapshot, ForgejoRunnerRef};
use shaula_core::registry::{FleetObservation, FleetObservationPhase};

use super::{access_reason, is_owned_runner, ForgejoPoolSupervisor, ForgejoReconcileReport};

impl ForgejoPoolSupervisor {
    pub(super) async fn observe_demand(
        &self,
        deleting: bool,
        now: i64,
    ) -> CoreResult<ForgejoReconcileReport> {
        let mut report = ForgejoReconcileReport::default();
        if deleting {
            return Ok(report);
        }
        match self.forgejo.list_jobs(&self.labels).await {
            Ok(jobs) => {
                let snapshot = ForgejoDemandSnapshot::from_jobs(jobs, now);
                let _claim = self.gates.acquire_claim(&self.fleet_key).await;
                if self
                    .current_head()
                    .await?
                    .is_none_or(|head| head.deletion_marker)
                {
                    report.stale_demand = true;
                    report.reason = Some(ReasonCode::OwnershipProofFailed);
                } else {
                    self.lifecycle
                        .demand_snapshot(
                            &self.fleet_key,
                            i64::try_from(snapshot.waiting_jobs).unwrap_or(i64::MAX),
                            now,
                        )
                        .await?;
                    report.waiting_jobs = snapshot.waiting_jobs;
                    report.running_jobs = snapshot.running_jobs;
                }
            }
            Err(failure) => {
                // Do not rewrite demand_snapshot: that would give old data a
                // fresh timestamp. Staleness is a separate status condition.
                report.waiting_jobs = u64::try_from(
                    self.store
                        .demand_get(&self.fleet_key)
                        .await?
                        .unwrap_or(0)
                        .max(0),
                )
                .unwrap_or(0);
                report.stale_demand = true;
                report.reason = Some(access_reason(&failure));
            }
        }
        report.target = self.target(report.waiting_jobs);
        Ok(report)
    }

    pub(super) async fn classify_inventory(
        &self,
        runners: &[ForgejoRunnerRef],
        report: &mut ForgejoReconcileReport,
    ) -> CoreResult<()> {
        let mut identities = std::collections::BTreeMap::new();
        for generation in self
            .lifecycle
            .generations_for_fleet(&self.fleet_key)
            .await?
        {
            if generation.state.counts_occupancy() {
                if let Some((id, uuid)) = self
                    .lifecycle
                    .generation_forgejo_runner(&generation.id)
                    .await?
                {
                    identities.insert(id, (uuid, generation.runner_name));
                }
            }
        }
        for runner in runners
            .iter()
            .filter(|r| r.name.starts_with(&self.runner_name_prefix))
        {
            let retained = i64::try_from(runner.id)
                .ok()
                .and_then(|id| identities.get(&id))
                .is_some_and(|(uuid, name)| *uuid == runner.uuid && *name == runner.name);
            if retained && is_owned_runner(runner, &self.labels) {
                report.owned_runners = report.owned_runners.saturating_add(1);
                report.idle_runners += u64::from(runner.is_idle());
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
        report: &ForgejoReconcileReport,
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
                if self
                    .current_head()
                    .await?
                    .is_some_and(|head| head.deletion_marker)
                    && self.store.generations_occupancy(&self.fleet_key).await? == 0
                    && self.forgejo.list_runners().await.is_ok_and(|runners| {
                        !runners
                            .iter()
                            .any(|r| r.name.starts_with(&self.runner_name_prefix))
                    })
                {
                    self.lifecycle
                        .fleet_set_tombstone(&self.fleet_key, now)
                        .await?;
                }
            }
            return Ok(());
        }
        let quarantined = generations.iter().any(|g| g.state.is_quarantined());
        let converging = generations.iter().any(|g| {
            g.state.counts_occupancy()
                && (g.fleet_revision != self.guard.desired_revision
                    || !matches!(
                        g.state,
                        shaula_core::lifecycle::GenerationState::Idle
                            | shaula_core::lifecycle::GenerationState::Busy
                    ))
        });
        let reason = report
            .reason
            .or(quarantined.then_some(ReasonCode::OwnershipProofFailed));
        let phase = if report.stale_demand || report.stale_inventory || reason.is_some() {
            FleetObservationPhase::Degraded
        } else if !converging
            && generations
                .iter()
                .filter(|g| g.state.counts_occupancy())
                .count() as u64
                == report.target
            && report.idle_runners.saturating_add(report.active_runners) == report.target
        {
            FleetObservationPhase::Ready
        } else {
            FleetObservationPhase::Reconciling
        };
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
