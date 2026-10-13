//! A separate, level-triggered GitLab Pool driver (spec 0043). It shares the
//! Runner Operation seam and the Pool create stages with Forgejo; demand,
//! claim detection and pausing are GitLab-specific.
//!
//! Failure modes the tick must contain:
//! 1. No proven anchor (missing, drifted, unpaused) → demand stale, no scale-down.
//! 2. Inventory read fails → readiness/effects wait; nothing is assumed absent.
//! 3. An unknown registration under the prefix → no creates, Degraded.
//! 4. A claim between ticks → association persisted before the pause.
//! 5. A token-holding process after its job → paused, then retired.
//! 6. Deleting Fleet → anchors and generations removed before the tombstone.

use std::sync::Arc;

use shaula_core::capacity::CapacityPolicy;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::gitlab::{FleetGitlabSection, GitlabMatching};
use shaula_core::ports::gitlab::GitlabPoolPort;
use shaula_core::ports::{ApplyIntentSink, Clock, TemplateRuntimePort};
use shaula_core::registry::{ControlPlaneStore, FleetRuntimeGuard, LifecycleStore};

use crate::pool_driver::{access_reason, create_count};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitlabReconcileReport {
    pub pending_jobs: u64,
    pub target: u64,
    pub owned_runners: u64,
    pub idle_runners: u64,
    pub active_runners: u64,
    pub missing_runners: u64,
    pub surplus_runners: u64,
    pub created: u32,
    pub destroyed: u32,
    pub unknown_runners: u64,
    pub stale_demand: bool,
    pub stale_inventory: bool,
    pub reason: Option<ReasonCode>,
}

/// Does not implement GitHub handoff, route proof, scale sets or listeners.
pub struct GitlabPoolSupervisor {
    diagnostics: Option<shaula_core::diagnostics::Observer>,
    fleet_key: String,
    guard: FleetRuntimeGuard,
    auth: (String, i64),
    capacity: CapacityPolicy,
    runner_name_prefix: String,
    matching: GitlabMatching,
    instance_url: String,
    store: Arc<dyn ControlPlaneStore>,
    lifecycle: Arc<dyn LifecycleStore>,
    gitlab: Arc<dyn GitlabPoolPort>,
    clock: Arc<dyn Clock>,
    runtime: Arc<dyn TemplateRuntimePort>,
    apply_intent_sink: Arc<dyn ApplyIntentSink>,
    gates: Arc<crate::effect_gate::FleetEffectGates>,
    tick_lock: tokio::sync::Mutex<()>,
    create_limit: Arc<tokio::sync::Semaphore>,
    destroy_limit: Arc<tokio::sync::Semaphore>,
    template_create: crate::template_create_gate::TemplateCreateGates,
    work_root: std::path::PathBuf,
    artifact_root: std::path::PathBuf,
    operation_timeout: std::time::Duration,
    runner_max_lifetime: std::time::Duration,
    setup_info_issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
}

pub struct GitlabPoolSupervisorDeps {
    pub guard: FleetRuntimeGuard,
    pub auth: (String, i64),
    pub store: Arc<dyn ControlPlaneStore>,
    pub lifecycle: Arc<dyn LifecycleStore>,
    pub gitlab: Arc<dyn GitlabPoolPort>,
    pub clock: Arc<dyn Clock>,
    pub runtime: Arc<dyn TemplateRuntimePort>,
    pub apply_intent_sink: Arc<dyn ApplyIntentSink>,
    pub gates: Arc<crate::effect_gate::FleetEffectGates>,
    pub create_limit: Arc<tokio::sync::Semaphore>,
    pub destroy_limit: Arc<tokio::sync::Semaphore>,
    pub template_create: crate::template_create_gate::TemplateCreateGates,
    pub work_root: std::path::PathBuf,
    pub artifact_root: std::path::PathBuf,
    pub operation_timeout: std::time::Duration,
    pub setup_info_issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
}

impl GitlabPoolSupervisor {
    pub fn new(
        fleet_key: impl Into<String>,
        section: &FleetGitlabSection,
        capacity: CapacityPolicy,
        deps: GitlabPoolSupervisorDeps,
    ) -> CoreResult<Self> {
        section.validate()?;
        capacity
            .validate()
            .map_err(|message| CoreError::new(ReasonCode::SpecInvalid, message))?;
        let fleet_key = fleet_key.into();
        let diagnostics = shaula_core::diagnostics::Observer::register(
            deps.store.diagnostic_sink(),
            shaula_core::diagnostics::Guard::fleet(&fleet_key, &deps.guard),
            shaula_core::diagnostics::Lane::Supervisor,
            shaula_core::diagnostics::QuestionId::ScaleUp,
        );
        Ok(Self {
            diagnostics,
            fleet_key,
            guard: deps.guard,
            auth: deps.auth,
            capacity,
            runner_name_prefix: section.runner_name_prefix.clone(),
            matching: section.matching(),
            instance_url: section.instance_url.clone(),
            store: deps.store,
            lifecycle: deps.lifecycle,
            gitlab: deps.gitlab,
            clock: deps.clock,
            runtime: deps.runtime,
            apply_intent_sink: deps.apply_intent_sink,
            gates: deps.gates,
            tick_lock: tokio::sync::Mutex::new(()),
            create_limit: deps.create_limit,
            destroy_limit: deps.destroy_limit,
            template_create: deps.template_create,
            work_root: deps.work_root,
            artifact_root: deps.artifact_root,
            operation_timeout: deps.operation_timeout,
            runner_max_lifetime: shaula_core::runner_lifetime::DEFAULT_MAX_LIFETIME,
            setup_info_issuer: deps.setup_info_issuer,
        })
    }

    #[must_use]
    pub fn with_runner_max_lifetime(mut self, limit: std::time::Duration) -> Self {
        self.runner_max_lifetime = limit;
        self
    }

    /// Unproven demand never authorizes ordinary scale-down. Explicit hard
    /// lifetime expiry is independent of demand freshness.
    pub async fn tick(&self) -> CoreResult<GitlabReconcileReport> {
        let _tick = self.tick_lock.lock().await;
        let now = self.clock.now_unix_ms();
        let mut diagnostic = self.diagnostics.as_ref().and_then(|o| o.begin(now));
        let result = self.tick_observed(now, &mut diagnostic).await;
        crate::diagnostic_capture::finish(diagnostic, result.is_err());
        result
    }

    async fn tick_observed(
        &self,
        now: i64,
        diagnostic: &mut Option<shaula_core::diagnostics::ObservationTicket>,
    ) -> CoreResult<GitlabReconcileReport> {
        use crate::diagnostic_capture::{note, passed};
        use shaula_core::diagnostics::*;
        let Some(head) = self.current_head().await? else {
            return Ok(GitlabReconcileReport {
                stale_demand: true,
                stale_inventory: true,
                reason: Some(ReasonCode::OwnershipProofFailed),
                ..Default::default()
            });
        };
        let deleting = head.deletion_marker;
        passed(diagnostic, StageId::Authority);
        if deleting {
            note(
                diagnostic,
                Code::ControlDecommissioning,
                StageId::Authority,
                false,
            );
        }
        let expired = self.expire_runners(now).await?;
        let runners = match self.gitlab.list_runners(&self.runner_name_prefix).await {
            Ok(runners) => runners,
            Err(failure) => {
                note(
                    diagnostic,
                    Code::ControlInventoryUnavailable,
                    StageId::Demand,
                    true,
                );
                let report = GitlabReconcileReport {
                    destroyed: expired.destroyed,
                    stale_demand: true,
                    stale_inventory: true,
                    reason: Some(access_reason(&failure)),
                    ..Default::default()
                };
                self.observe_status(&report, deleting, now).await?;
                return Ok(report);
            }
        };
        let anchor = self.reconcile_anchors(&runners, deleting, now).await?;
        let mut report = self.observe_demand(anchor, deleting, now).await?;
        report.destroyed = expired.destroyed;
        if report.stale_demand {
            note(
                diagnostic,
                Code::ControlDemandUnavailable,
                StageId::Demand,
                true,
            );
        } else {
            passed(diagnostic, StageId::Demand);
        }
        self.classify_inventory(&runners, &mut report).await?;
        self.reconcile_generation_readiness(&runners, now).await?;
        if !report.stale_demand {
            report.destroyed += self.destroy_excess(report.target, now).await?.destroyed;
            if !deleting && report.unknown_runners == 0 {
                let (effective, occupancy) = self.store.capacity_counters(&self.fleet_key).await?;
                crate::diagnostic_capture::capacity_decision(
                    diagnostic,
                    self.capacity,
                    Some(i64::try_from(report.pending_jobs).unwrap_or(i64::MAX)),
                    shaula_core::capacity::CapacityCounters {
                        effective_capacity: effective,
                        resource_occupancy: occupancy,
                    },
                    DemandKind::GitlabPendingJobs,
                    Some(now),
                );
                let creates = create_count(self.capacity, report.target, effective, occupancy);
                for _ in 0..creates {
                    if self.create_one_generation(now).await? {
                        report.created = report.created.saturating_add(1);
                    } else {
                        break;
                    }
                }
            }
        }
        report.missing_runners = report.target.saturating_sub(
            report
                .owned_runners
                .saturating_add(u64::from(report.created)),
        );
        self.observe_status(&report, deleting, now).await?;
        Ok(report)
    }

    async fn current_head(&self) -> CoreResult<Option<shaula_core::registry::FleetHead>> {
        Ok(self
            .store
            .fleet_get(&self.fleet_key)
            .await?
            .filter(|head| !head.tombstone && FleetRuntimeGuard::from(head) == self.guard))
    }

    fn target(&self, pending_jobs: u64) -> u64 {
        crate::pool_driver::target(self.capacity, pending_jobs)
    }
}

#[path = "gitlab_supervisor_anchor.rs"]
mod anchor;
#[path = "gitlab_supervisor_destroy.rs"]
mod destroy;
#[path = "gitlab_supervisor_generation.rs"]
mod generation;
#[path = "gitlab_supervisor_observation.rs"]
mod observation;
#[path = "gitlab_supervisor_readiness.rs"]
mod readiness;
#[path = "gitlab_supervisor_registration.rs"]
mod registration;
