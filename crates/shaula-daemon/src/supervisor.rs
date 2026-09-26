//! Per-fleet supervision: ownership reconciliation, capacity convergence
//! and the provider-neutral runner lifecycle. One tick is one pass of the
//! level-triggered reconcile; every external effect is preceded by a
//! durable intent and followed by a durable result.

use std::sync::Arc;

use futures::future::join_all;
use sha2::Digest;
use shaula_core::capacity::{create_count, target, AssignedDemand, CapacityCounters};
use shaula_core::error::CoreResult;
use shaula_core::ports::{GitHubAccessPort, LookupOutcome, TemplateRuntimePort};
use shaula_core::registry::LifecycleStore;

pub struct FleetSupervisor {
    diagnostics: Option<shaula_core::diagnostics::Observer>,
    runner_max_lifetime: std::time::Duration,
    clock: Option<Arc<dyn shaula_core::ports::Clock>>,
    limits: LifecycleLimits,
    store: Arc<dyn LifecycleStore>,
    github: Arc<dyn GitHubAccessPort>,
    handoff_github: Arc<dyn GitHubAccessPort>,
    handoff_authority: (String, i64),
    execution_ready: bool,
    revision_clients: std::collections::HashMap<(String, i64), Arc<dyn GitHubAccessPort>>,
    runtime: Arc<dyn TemplateRuntimePort>,
    handoff: Arc<dyn shaula_core::registry::ControlPlaneStore>,
    config: FleetSupervisorConfig,
    identity: shaula_core::github::ScaleSetIdentity,
    runtime_guard: Option<shaula_core::registry::FleetRuntimeGuard>,
    listener: Option<Arc<crate::listener::FleetListener>>,
    setup_info_issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
}

impl FleetSupervisor {
    /// One level-triggered reconcile pass.
    pub async fn tick(&self, now: i64) -> CoreResult<ReconcileReport> {
        let mut diagnostic = self.diagnostics.as_ref().and_then(|o| o.begin(now));
        let epoch = self
            .store
            .session_get(&self.config.fleet_key)
            .await?
            .map(|s| s.epoch);
        let result = self.reconcile(now, epoch, &mut diagnostic).await;
        crate::diagnostic_capture::finish(diagnostic, result.is_err());
        self.observe_reconcile(&result, epoch, now).await?;
        result
    }

    async fn reconcile(
        &self,
        now: i64,
        epoch: Option<i64>,
        diagnostic: &mut Option<shaula_core::diagnostics::ObservationTicket>,
    ) -> CoreResult<ReconcileReport> {
        use crate::diagnostic_capture::{note, passed};
        use shaula_core::diagnostics::{Code, StageId};
        if let Some(d) = diagnostic {
            d.observation.guard.session_epoch = Some(epoch);
        }
        let mut report = ReconcileReport {
            session_epoch: epoch,
            ..ReconcileReport::default()
        };
        let head = self.handoff.fleet_get(&self.config.fleet_key).await?;
        if let Some(guard) = &self.runtime_guard {
            if head.as_ref().is_none_or(|head| {
                head.tombstone || shaula_core::registry::FleetRuntimeGuard::from(head) != *guard
            }) {
                tracing::debug!(fleet = %self.config.fleet_key, "reconcile skipped: runtime guard mismatch or tombstoned");
                return Ok(report);
            }
        }

        // Hard lifetime is independent of demand, listener health and auth handoff.
        // It destroys only resources proved by this generation's original pins.
        let expired = self.expire_runners(now).await?;
        report.destroyed = expired.destroyed;
        report.quarantined = expired.quarantined;

        // 1. Auth handoff: read-only classification then acknowledge.
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
        report.handoff_acknowledged = matches!(
            progress,
            crate::handoff::HandoffProgress::Acknowledged
                | crate::handoff::HandoffProgress::UpToDate
        );
        if matches!(
            progress,
            crate::handoff::HandoffProgress::Blocked
                | crate::handoff::HandoffProgress::Acknowledged
                | crate::handoff::HandoffProgress::Retry
        ) {
            // Blocked: quiesced until classification (0002 §7.5).
            // Acknowledged (G3): the JUST-acknowledged context must be
            // re-read by wiring before any new effect runs — this tick
            // stops after settlement and reconciles from the acknowledged
            // snapshot on the next pass.
            // Retry (G5): a stale CAS is NOT settlement — abort this tick
            // and retry from current authority, never continue effects.
            match progress {
                crate::handoff::HandoffProgress::Blocked => {
                    report.blocked = true;
                    report.reason = Some(shaula_core::error::ReasonCode::OwnershipProofFailed);
                }
                crate::handoff::HandoffProgress::Acknowledged => {
                    report.settled_this_tick = true;
                }
                _ => {}
            }
            note(
                diagnostic,
                Code::ControlAuthContextPending,
                StageId::Authority,
                true,
            );
            return Ok(report);
        }

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
            report.blocked = true;
            note(
                diagnostic,
                Code::ControlAuthContextPending,
                StageId::Authority,
                true,
            );
            return Ok(report);
        }

        if let Some(d) = diagnostic {
            d.observation.guard.auth = Some(execution_ref);
        }

        if head.is_some_and(|head| head.deletion_marker) {
            note(
                diagnostic,
                Code::ControlDecommissioning,
                StageId::Authority,
                false,
            );
            if let Some(listener) = &self.listener {
                listener.stop().await?;
            }
            let retired = self.retire_excess(i64::MAX, now).await?;
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
            return Ok(report);
        }

        // 2. Ownership: create-or-adopt with persist-before-POST semantics.
        match self.ensure_ownership(now).await? {
            OwnershipOutcome::Ready => report.scale_set_bound = true,
            OwnershipOutcome::Blocked(reason) => {
                note(
                    diagnostic,
                    if reason == shaula_core::error::ReasonCode::RateLimited {
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
                return Ok(report);
            }
        }
        passed(diagnostic, StageId::Authority);
        if let Some(listener) = &self.listener {
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
                return Ok(report);
            }
        }

        // 2.5 Readiness reconciliation (spec 0024): WaitingOnline
        // generations join Idle/cleanup before this tick's capacity pass,
        // so a completed ephemeral runner frees its slot immediately.
        self.reconcile_generation_readiness(now, report.session_epoch)
            .await?;

        // 3. Capacity convergence.
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
        let creates = create_count(
            &self.config.capacity,
            &AssignedDemand {
                total_assigned_jobs: demand,
            },
            &counters,
        );
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
        let current_target = target(
            &self.config.capacity,
            &AssignedDemand {
                total_assigned_jobs: demand,
            },
        );

        let excess = (effective - current_target).max(0);
        let retired = self.retire_excess(excess, now).await?;
        report.destroyed += retired.destroyed;
        report.quarantined += retired.quarantined;

        // Start the whole deficit together. Each operation acquires the
        // shared create semaphore inside `create_one_generation`, so this
        // preserves the global create budget while allowing independent
        // workspaces to make progress concurrently. Wait for every started
        // operation before propagating an error so one failure does not
        // cancel sibling creates that may already have crossed an effect
        // boundary.
        let results = join_all((0..creates).map(|_| self.create_one_generation(now))).await;
        for result in results {
            if result? {
                report.created += 1;
            }
        }

        Ok(report)
    }
}

pub(crate) fn fingerprint(identity: &shaula_core::github::ScaleSetIdentity) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(identity.target.config_url().as_bytes());
    hasher.update(identity.runner_group.as_bytes());
    hasher.update(identity.scale_set_name.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[path = "supervisor_labels.rs"]
mod labels_impl;
#[path = "supervisor_ownership.rs"]
mod ownership_impl;

#[path = "supervisor_cleanup.rs"]
mod cleanup_impl;
#[path = "supervisor_health.rs"]
mod health_impl;

#[path = "supervisor_status.rs"]
mod status_impl;

#[path = "supervisor_lifecycle.rs"]
mod lifecycle_impl;

#[path = "supervisor_readiness.rs"]
mod readiness_impl;
pub use readiness_impl::READINESS_TIMEOUT_MS;

#[path = "supervisor_destroy.rs"]
mod destroy_impl;

#[path = "supervisor_types.rs"]
mod types;
pub use types::{FleetSupervisorConfig, FleetSupervisorDeps, LifecycleLimits, ReconcileReport};

#[path = "supervisor_ownership_outcome.rs"]
mod ownership_outcome;
pub(crate) use ownership_outcome::OwnershipOutcome;

#[path = "supervisor_config.rs"]
mod config_impl;
