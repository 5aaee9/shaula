//! A separate, level-triggered Forgejo Pool driver. Registration and Terraform
//! effects have independent durable evidence; neither is replayed on restart.

use std::sync::Arc;

use shaula_core::capacity::CapacityPolicy;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetForgejoSection;
use shaula_core::ports::forgejo::{ForgejoPoolPort, ForgejoRunnerRef};
use shaula_core::ports::{AccessFailure, ApplyIntentSink, Clock, TemplateRuntimePort};
use shaula_core::registry::{ControlPlaneStore, FleetRuntimeGuard, LifecycleStore};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForgejoReconcileReport {
    pub waiting_jobs: u64,
    pub running_jobs: u64,
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
pub struct ForgejoPoolSupervisor {
    fleet_key: String,
    guard: FleetRuntimeGuard,
    auth: (String, i64),
    capacity: CapacityPolicy,
    runner_name_prefix: String,
    labels: Vec<String>,
    bootstrap_labels: Vec<String>,
    instance_url: String,
    store: Arc<dyn ControlPlaneStore>,
    lifecycle: Arc<dyn LifecycleStore>,
    forgejo: Arc<dyn ForgejoPoolPort>,
    clock: Arc<dyn Clock>,
    runtime: Arc<dyn TemplateRuntimePort>,
    apply_intent_sink: Arc<dyn ApplyIntentSink>,
    gates: Arc<crate::effect_gate::FleetEffectGates>,
    tick_lock: tokio::sync::Mutex<()>,
    create_limit: Arc<tokio::sync::Semaphore>,
    destroy_limit: Arc<tokio::sync::Semaphore>,
    work_root: std::path::PathBuf,
    artifact_root: std::path::PathBuf,
    operation_timeout: std::time::Duration,
    setup_info_issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
}

pub struct ForgejoPoolSupervisorDeps {
    pub guard: FleetRuntimeGuard,
    pub auth: (String, i64),
    pub store: Arc<dyn ControlPlaneStore>,
    pub lifecycle: Arc<dyn LifecycleStore>,
    pub forgejo: Arc<dyn ForgejoPoolPort>,
    pub clock: Arc<dyn Clock>,
    pub runtime: Arc<dyn TemplateRuntimePort>,
    pub apply_intent_sink: Arc<dyn ApplyIntentSink>,
    pub gates: Arc<crate::effect_gate::FleetEffectGates>,
    pub create_limit: Arc<tokio::sync::Semaphore>,
    pub destroy_limit: Arc<tokio::sync::Semaphore>,
    pub work_root: std::path::PathBuf,
    pub artifact_root: std::path::PathBuf,
    pub operation_timeout: std::time::Duration,
    pub setup_info_issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
}

impl ForgejoPoolSupervisor {
    pub fn new(
        fleet_key: impl Into<String>,
        section: &FleetForgejoSection,
        capacity: CapacityPolicy,
        deps: ForgejoPoolSupervisorDeps,
    ) -> CoreResult<Self> {
        section.validate()?;
        capacity
            .validate()
            .map_err(|message| CoreError::new(ReasonCode::SpecInvalid, message))?;
        Ok(Self {
            fleet_key: fleet_key.into(),
            guard: deps.guard,
            auth: deps.auth,
            capacity,
            runner_name_prefix: section.runner_name_prefix.clone(),
            labels: normalized_labels(&section.labels)?,
            bootstrap_labels: section.labels.clone(),
            instance_url: section.instance_url.clone(),
            store: deps.store,
            lifecycle: deps.lifecycle,
            forgejo: deps.forgejo,
            clock: deps.clock,
            runtime: deps.runtime,
            apply_intent_sink: deps.apply_intent_sink,
            gates: deps.gates,
            tick_lock: tokio::sync::Mutex::new(()),
            create_limit: deps.create_limit,
            destroy_limit: deps.destroy_limit,
            work_root: deps.work_root,
            artifact_root: deps.artifact_root,
            operation_timeout: deps.operation_timeout,
            setup_info_issuer: deps.setup_info_issuer,
        })
    }

    /// Poll failures never replace the durable demand or authorize scale-down.
    pub async fn tick(&self) -> CoreResult<ForgejoReconcileReport> {
        let _tick = self.tick_lock.lock().await;
        let now = self.clock.now_unix_ms();
        let Some(head) = self.current_head().await? else {
            return Ok(ForgejoReconcileReport {
                stale_demand: true,
                stale_inventory: true,
                reason: Some(ReasonCode::OwnershipProofFailed),
                ..Default::default()
            });
        };
        let deleting = head.deletion_marker;
        let mut report = self.observe_demand(deleting, now).await?;
        let runners = match self.forgejo.list_runners().await {
            Ok(runners) => runners,
            Err(failure) => {
                report.stale_inventory = true;
                report.reason.get_or_insert(access_reason(&failure));
                self.observe_status(&report, deleting, now).await?;
                return Ok(report);
            }
        };
        self.classify_inventory(&runners, &mut report).await?;
        // Inventory can update readiness on its own. Only a fully fresh pass
        // may authorize resource effects, including recovery/scale-down.
        self.reconcile_generation_readiness(&runners, now).await?;
        if !report.stale_demand {
            // Never use a pre-Create inventory to clean up a new registration.
            report.destroyed = self.destroy_excess(report.target, now).await?;
            if !deleting && report.unknown_runners == 0 {
                let (effective, occupancy) = self.store.capacity_counters(&self.fleet_key).await?;
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

    fn target(&self, waiting_jobs: u64) -> u64 {
        u64::try_from(self.capacity.min_runners)
            .unwrap_or(0)
            .saturating_add(waiting_jobs)
            .min(u64::try_from(self.capacity.max_runners).unwrap_or(0))
    }
}

fn create_count(capacity: CapacityPolicy, target: u64, effective: i64, occupancy: i64) -> u64 {
    target
        .saturating_sub(u64::try_from(effective).unwrap_or(u64::MAX))
        .min(u64::try_from(capacity.max_runners.saturating_sub(occupancy)).unwrap_or(0))
}

fn normalized_labels(labels: &[String]) -> CoreResult<Vec<String>> {
    shaula_core::forgejo::validate_labels(labels)?;
    Ok(labels
        .iter()
        .map(|label| label.split(':').next().unwrap_or_default().to_string())
        .collect())
}

fn is_owned_runner(runner: &ForgejoRunnerRef, labels: &[String]) -> bool {
    runner.ephemeral
        && runner.is_known()
        && labels.iter().all(|label| runner.labels.contains(label))
}

fn access_reason(failure: &AccessFailure) -> ReasonCode {
    match failure {
        AccessFailure::Unauthenticated => ReasonCode::Unauthenticated,
        AccessFailure::PermissionDenied => ReasonCode::PermissionDenied,
        AccessFailure::TargetHiddenOrNotFound => ReasonCode::TargetHiddenOrNotFound,
        AccessFailure::RateLimited { .. } => ReasonCode::RateLimited,
        _ => ReasonCode::AccessVerificationFailed,
    }
}

#[path = "forgejo_supervisor_destroy.rs"]
mod destroy;
#[path = "forgejo_supervisor_generation.rs"]
mod generation;
#[path = "forgejo_supervisor_observation.rs"]
mod observation;
#[path = "forgejo_supervisor_readiness.rs"]
mod readiness;
#[path = "forgejo_supervisor_registration.rs"]
mod registration;
#[cfg(test)]
#[path = "forgejo_supervisor_tests.rs"]
mod tests;
