//! The level-triggered reconcile pass of [`FleetSupervisor`], split into
//! named stages. Stages that may stop the tick return `false` after having
//! recorded the stop on the report and diagnostic.

use futures::future::join_all;
use shaula_core::capacity::{create_count, target, AssignedDemand, CapacityCounters};
use shaula_core::diagnostics::{Code, ObservationTicket, StageId};
use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::registry::{FleetHead, FleetRuntimeGuard};

use crate::diagnostic_capture::{note, passed};
use crate::handoff::HandoffProgress;

use super::{FleetSupervisor, OwnershipOutcome, ReconcileReport};

type Diagnostic = Option<ObservationTicket>;

/// Whether a configured runtime guard still admits the fleet head. No
/// configured guard always admits.
fn runtime_guard_admits(guard: Option<&FleetRuntimeGuard>, head: Option<&FleetHead>) -> bool {
    match guard {
        Some(guard) => {
            !head.is_none_or(|head| head.tombstone || FleetRuntimeGuard::from(head) != *guard)
        }
        None => true,
    }
}

/// Applies a handoff outcome to the report; returns `true` when the tick
/// must stop after settlement.
fn handoff_halts(progress: HandoffProgress, report: &mut ReconcileReport) -> bool {
    report.handoff_acknowledged = matches!(
        progress,
        HandoffProgress::Acknowledged | HandoffProgress::UpToDate
    );
    // Blocked: quiesced until classification (0002 §7.5).
    // Acknowledged (G3): the JUST-acknowledged context must be
    // re-read by wiring before any new effect runs — this tick
    // stops after settlement and reconciles from the acknowledged
    // snapshot on the next pass.
    // Retry (G5): a stale CAS is NOT settlement — abort this tick
    // and retry from current authority, never continue effects.
    match progress {
        HandoffProgress::Blocked => {
            report.blocked = true;
            report.reason = Some(ReasonCode::OwnershipProofFailed);
            true
        }
        HandoffProgress::Acknowledged => {
            report.settled_this_tick = true;
            true
        }
        HandoffProgress::Retry => true,
        HandoffProgress::UpToDate => false,
    }
}

/// Create/retire decision derived from demand and current capacity.
struct CapacityPlan {
    creates: i64,
    excess: i64,
}

impl FleetSupervisor {
    pub(super) async fn reconcile(
        &self,
        now: i64,
        epoch: Option<i64>,
        diagnostic: &mut Diagnostic,
    ) -> CoreResult<ReconcileReport> {
        if let Some(d) = diagnostic {
            d.observation.guard.session_epoch = Some(epoch);
        }
        let mut report = ReconcileReport {
            session_epoch: epoch,
            ..ReconcileReport::default()
        };
        let head = self.handoff.fleet_get(&self.config.fleet_key).await?;
        let current_revision = head.as_ref().map(|head| head.desired_revision);
        if !runtime_guard_admits(self.runtime_guard.as_ref(), head.as_ref()) {
            tracing::debug!(fleet = %self.config.fleet_key, "reconcile skipped: runtime guard mismatch or tombstoned");
            return Ok(report);
        }

        // Hard lifetime is independent of demand, listener health and auth handoff.
        // It destroys only resources proved by this generation's original pins.
        let expired = self.expire_runners(now).await?;
        report.destroyed = expired.destroyed;
        report.quarantined = expired.quarantined;

        // 1. Auth handoff: read-only classification then acknowledge.
        let progress = self.run_auth_handoff(now).await?;
        if handoff_halts(progress, &mut report) {
            note(
                diagnostic,
                Code::ControlAuthContextPending,
                StageId::Authority,
                true,
            );
            return Ok(report);
        }
        let Some(execution_ref) = self.ready_execution_ref().await? else {
            report.blocked = true;
            note(
                diagnostic,
                Code::ControlAuthContextPending,
                StageId::Authority,
                true,
            );
            return Ok(report);
        };
        if let Some(d) = diagnostic {
            d.observation.guard.auth = Some(execution_ref);
        }

        if head.is_some_and(|head| head.deletion_marker) {
            self.decommission(now, &mut report, diagnostic).await?;
            return Ok(report);
        }

        // 2. Ownership: create-or-adopt with persist-before-POST semantics.
        if !self.ownership_stage(now, &mut report, diagnostic).await? {
            return Ok(report);
        }
        passed(diagnostic, StageId::Authority);
        if !self.listener_stage(&mut report, diagnostic).await? {
            return Ok(report);
        }

        // 2.5 Readiness reconciliation (spec 0024): WaitingOnline
        // generations join Idle/cleanup before this tick's capacity pass,
        // so a completed ephemeral runner frees its slot immediately.
        self.reconcile_generation_readiness(now, report.session_epoch)
            .await?;

        // 3. Capacity convergence.
        let plan = self.capacity_plan(diagnostic).await?;
        let retired = self
            .retire_excess(plan.excess, current_revision, now)
            .await?;
        report.destroyed += retired.destroyed;
        report.quarantined += retired.quarantined;
        self.run_creates(plan.creates, now, &mut report).await?;

        Ok(report)
    }

    /// Runs the auth handoff while holding the listener's handoff gate.
    async fn run_auth_handoff(&self, now: i64) -> CoreResult<HandoffProgress> {
        let bound_scale_set_id = self
            .store
            .scale_set_get(&self.config.fleet_key)
            .await?
            .and_then(|s| s.scale_set_id);
        let handoff_gate = match &self.listener {
            Some(listener) => Some(listener.handoff_gate().await),
            None => None,
        };
        let progress = crate::handoff::run_handoff(
            &self.handoff,
            &self.config.fleet_key,
            &self.handoff_github,
            &self.handoff_authority,
            &self.identity,
            bound_scale_set_id,
            now,
        )
        .await?;
        drop(handoff_gate);
        Ok(progress)
    }

    /// The execution Auth Revision Ref when this supervisor's context is
    /// ready and matches the observed handoff; `None` blocks the tick.
    async fn ready_execution_ref(&self) -> CoreResult<Option<(String, i64)>> {
        let execution_ref = (
            self.config.auth_profile_key.clone(),
            self.config.auth_revision,
        );
        let observed = self
            .handoff
            .handoff_get(&self.config.fleet_key)
            .await?
            .and_then(|handoff| handoff.observed);
        if !self.execution_ready || observed.as_ref() != Some(&execution_ref) {
            tracing::debug!(fleet = %self.config.fleet_key, execution_ready = self.execution_ready, observed_matches = observed.as_ref() == Some(&execution_ref), "reconcile blocked: execution context not ready");
            return Ok(None);
        }
        Ok(Some(execution_ref))
    }

    /// Decommission pass: stop the listener, retire everything and report
    /// whether every owned generation is terminal.
    async fn decommission(
        &self,
        now: i64,
        report: &mut ReconcileReport,
        diagnostic: &mut Diagnostic,
    ) -> CoreResult<()> {
        note(
            diagnostic,
            Code::ControlDecommissioning,
            StageId::Authority,
            false,
        );
        if let Some(listener) = &self.listener {
            listener.stop().await?;
        }
        let retired = self.retire_excess(i64::MAX, None, now).await?;
        report.destroyed += retired.destroyed;
        report.quarantined += retired.quarantined;
        // Completion predicate (spec 0002 §4.4): every owned
        // Generation terminal (Destroyed). A still-Retiring or
        // Quarantined generation keeps the Decommission Change
        // visibly non-terminal; it never reports complete.
        let generations = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?;
        report.decommission_complete = generations.iter().all(|g| g.state.is_terminal());
        Ok(())
    }

    /// Ownership create-or-adopt; `false` stops the tick (and the listener).
    async fn ownership_stage(
        &self,
        now: i64,
        report: &mut ReconcileReport,
        diagnostic: &mut Diagnostic,
    ) -> CoreResult<bool> {
        match self.ensure_ownership(now).await? {
            OwnershipOutcome::Ready => {
                report.scale_set_bound = true;
                Ok(true)
            }
            OwnershipOutcome::Blocked(reason) => {
                note(
                    diagnostic,
                    if reason == ReasonCode::RateLimited {
                        Code::ControlRateLimited
                    } else {
                        Code::ControlOwnershipUnproven
                    },
                    StageId::Authority,
                    true,
                );
                report.blocked = true;
                report.reason = Some(reason);
                if let Some(listener) = &self.listener {
                    if listener.stop().await? {
                        report.session_epoch = None;
                    }
                }
                Ok(false)
            }
        }
    }

    /// Ensures the listener session (when one is configured); `false`
    /// stops the tick because the listener is not ready.
    async fn listener_stage(
        &self,
        report: &mut ReconcileReport,
        diagnostic: &mut Diagnostic,
    ) -> CoreResult<bool> {
        let Some(listener) = &self.listener else {
            return Ok(true);
        };
        let id = self
            .store
            .scale_set_get(&self.config.fleet_key)
            .await?
            .and_then(|s| s.scale_set_id);
        let installed_epoch = match id {
            Some(id) => listener.ensure_session(id).await?,
            None => None,
        };
        report.listener_ready = installed_epoch.is_some();
        if installed_epoch.is_some() {
            report.session_epoch = installed_epoch;
            if let Some(d) = diagnostic {
                d.observation.guard.session_epoch = Some(installed_epoch);
            }
        }
        let listener_reason = listener.reason().await;
        if !report.listener_ready || listener_reason.is_some() {
            tracing::debug!(fleet = %self.config.fleet_key, listener_ready = report.listener_ready, reason = ?listener_reason, "reconcile blocked: listener not ready");
            report.blocked = true;
            report.reason = listener_reason;
            note(
                diagnostic,
                Code::ControlListenerNotReady,
                StageId::Demand,
                true,
            );
            return Ok(false);
        }
        Ok(true)
    }

    /// Reads demand and capacity, records the decision and derives how many
    /// generations to create and retire.
    async fn capacity_plan(&self, diagnostic: &mut Diagnostic) -> CoreResult<CapacityPlan> {
        let demand_evidence = self
            .handoff
            .demand_with_time(&self.config.fleet_key)
            .await?;
        let demand = demand_evidence.map(|(value, _)| value).unwrap_or(0);
        let (effective, occupancy) = self.capacity_counters().await?;
        let counters = CapacityCounters {
            effective_capacity: effective,
            resource_occupancy: occupancy,
        };
        if self.listener.is_none() {
            note(
                diagnostic,
                Code::ControlListenerNotReady,
                StageId::Demand,
                false,
            );
        } else {
            passed(diagnostic, StageId::Demand);
        }
        crate::diagnostic_capture::capacity_decision(
            diagnostic,
            self.config.capacity,
            demand_evidence.map(|v| v.0),
            counters,
            shaula_core::diagnostics::DemandKind::GithubTotalAssignedJobs,
            demand_evidence.and_then(|v| v.1),
        );
        let assigned = AssignedDemand {
            total_assigned_jobs: demand,
        };
        let creates = create_count(&self.config.capacity, &assigned, &counters);
        // Permanent ops telemetry: the create/retire decision inputs are
        // otherwise invisible when a fleet silently stalls (2026-09-10
        // incident: quarantined generations and missing transitions were
        // undiagnosable without them).
        tracing::info!(
            fleet = %self.config.fleet_key,
            demand,
            effective = counters.effective_capacity,
            occupancy = counters.resource_occupancy,
            creates,
            "capacity decision"
        );
        let current_target = target(&self.config.capacity, &assigned);
        Ok(CapacityPlan {
            creates,
            excess: (effective - current_target).max(0),
        })
    }

    /// Start the whole deficit together. Each operation acquires the
    /// shared create semaphore inside `create_one_generation`, so this
    /// preserves the global create budget while allowing independent
    /// workspaces to make progress concurrently. Wait for every started
    /// operation before propagating an error so one failure does not
    /// cancel sibling creates that may already have crossed an effect
    /// boundary.
    async fn run_creates(
        &self,
        creates: i64,
        now: i64,
        report: &mut ReconcileReport,
    ) -> CoreResult<()> {
        let results = join_all((0..creates).map(|_| self.create_one_generation(now))).await;
        for result in results {
            if result? {
                report.created += 1;
            }
        }
        Ok(())
    }
}
