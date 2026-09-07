//! Per-fleet supervision: ownership reconciliation, capacity convergence
//! and the provider-neutral runner lifecycle. One tick is one pass of the
//! level-triggered reconcile; every external effect is preceded by a
//! durable intent and followed by a durable result.

use std::sync::Arc;

use sha2::Digest;
use shaula_core::capacity::{
    create_count, target, AssignedDemand, CapacityCounters, CapacityPolicy,
};
use shaula_core::error::CoreResult;
use shaula_core::ports::{GitHubAccessPort, LookupOutcome, TemplateRuntimePort};
use shaula_core::registry::LifecycleStore;

/// Static per-fleet configuration frozen at supervisor construction.
#[derive(Clone)]
pub struct FleetSupervisorConfig {
    pub fleet_key: String,
    pub capacity: CapacityPolicy,
    pub work_root: std::path::PathBuf,
    pub operation_timeout: std::time::Duration,
    /// Content-addressed artifact root (profile.yaml read from here).
    pub artifact_root: std::path::PathBuf,
    /// Durable apply-start sink backing at-most-once applies.
    pub apply_intent_sink: std::sync::Arc<dyn shaula_core::ports::ApplyIntentSink>,
    /// Fleet labels; empty falls back to the scale-set-name System
    /// label matching the Go SDK default.
    pub labels: Vec<shaula_core::github::Label>,
    /// R10-09: the Auth Revision Ref this supervisor was admitted with —
    /// frozen into every JIT intent so recovery can prove WHICH
    /// admission-time authority minted the token.
    pub auth_profile_key: String,
    pub auth_revision: i64,
}

/// One reconcile pass outcome, for telemetry and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    pub handoff_acknowledged: bool,
    pub scale_set_bound: bool,
    pub created: u32,
    pub destroyed: u32,
    pub quarantined: u32,
    pub blocked: bool,
}

pub struct FleetSupervisor {
    limits: LifecycleLimits,
    store: Arc<dyn LifecycleStore>,
    github: Arc<dyn GitHubAccessPort>,
    runtime: Arc<dyn TemplateRuntimePort>,
    handoff: Arc<dyn shaula_core::registry::ControlPlaneStore>,
    config: FleetSupervisorConfig,
    identity: shaula_core::github::ScaleSetIdentity,
}

impl FleetSupervisor {
    /// Constructor dependencies. The generation's template pin and inputs
    pub fn new(
        deps: FleetSupervisorDeps,
        config: FleetSupervisorConfig,
        identity: shaula_core::github::ScaleSetIdentity,
    ) -> Self {
        let FleetSupervisorDeps {
            store,
            handoff,
            github,
            runtime,
            limits,
        } = deps;
        Self {
            limits,
            store,
            handoff,
            github,
            runtime,
            config,
            identity,
        }
    }

    /// One level-triggered reconcile pass.
    pub async fn tick(&self, now: i64) -> CoreResult<ReconcileReport> {
        let mut report = ReconcileReport::default();

        // 1. Auth handoff: read-only classification then acknowledge.
        let bound_scale_set_id = self
            .store
            .scale_set_get(&self.config.fleet_key)
            .await?
            .and_then(|s| s.scale_set_id);
        let progress = crate::handoff::run_handoff(
            &self.handoff,
            &self.config.fleet_key,
            &self.github,
            &self.identity,
            bound_scale_set_id,
            now,
            30,
        )
        .await?;
        report.handoff_acknowledged = !matches!(progress, crate::handoff::HandoffProgress::Blocked);
        if matches!(progress, crate::handoff::HandoffProgress::Blocked) {
            // Quiesced: acquisition, create-or-adopt and lifecycle effects
            // stay stopped until the handoff classifies (0002 §7.5). Never
            // fall back to the previous credential.
            report.blocked = true;
            return Ok(report);
        }

        // 2. Ownership: create-or-adopt with persist-before-POST semantics.
        let bound = self.ensure_ownership(now).await?;
        report.scale_set_bound = bound;
        if !bound {
            report.blocked = true;
            return Ok(report);
        }

        // 3. Capacity convergence.
        let demand = self
            .handoff
            .demand_get(&self.config.fleet_key)
            .await?
            .unwrap_or(0);
        let (effective, occupancy) = self.capacity_counters().await?;
        let counters = CapacityCounters {
            effective_capacity: effective,
            resource_occupancy: occupancy,
        };
        let creates = create_count(
            &self.config.capacity,
            &AssignedDemand {
                total_assigned_jobs: demand,
            },
            &counters,
        );
        let current_target = target(
            &self.config.capacity,
            &AssignedDemand {
                total_assigned_jobs: demand,
            },
        );

        let excess = (effective - current_target).max(0);
        report.destroyed = self.retire_excess(excess, now).await?;

        for _ in 0..creates {
            if self.create_one_generation(now).await? {
                report.created += 1;
            }
        }

        // Cleanup reconcile: a CleanupRequired generation whose old child
        // cannot be proven terminated never auto-destroys; it transitions
        // to Quarantined so an explicit, auditable operator procedure can
        // take over (spec 0001 §11.2 — no silent forget).
        report.quarantined = self.quarantine_stale_cleanup(now).await?;

        Ok(report)
    }

    /// Promotes CleanupRequired generations with no proven side effect to
    /// Quarantined after the cleanup deadline, keeping occupancy.
    async fn quarantine_stale_cleanup(&self, now: i64) -> CoreResult<u32> {
        let mut quarantined = 0u32;
        let generations = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?;
        for generation in generations {
            if generation.state != shaula_core::lifecycle::GenerationState::CleanupRequired {
                continue;
            }
            // CleanupRequired stays at least one full tick before
            // quarantining, giving a live cleanup attempt time to finish.
            if now - generation.updated_at < 60_000 {
                continue;
            }
            self.store
                .generation_advance(
                    &generation.id,
                    shaula_core::lifecycle::GenerationState::Quarantined,
                    now,
                )
                .await?;
            quarantined += 1;
        }
        Ok(quarantined)
    }

    async fn capacity_counters(&self) -> CoreResult<(i64, i64)> {
        let generations = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?;
        let mut effective = 0i64;
        let mut occupancy = 0i64;
        for generation in &generations {
            if generation.state.counts_effective() {
                effective += 1;
            }
            if generation.state.counts_occupancy() {
                occupancy += 1;
            }
        }
        Ok((effective, occupancy))
    }
}

pub(crate) fn fingerprint(identity: &shaula_core::github::ScaleSetIdentity) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(identity.target.config_url().as_bytes());
    hasher.update(identity.runner_group.as_bytes());
    hasher.update(identity.scale_set_name.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[path = "supervisor_ownership.rs"]
mod ownership_impl;

#[path = "supervisor_lifecycle.rs"]
mod lifecycle_impl;

#[path = "supervisor_destroy.rs"]
mod destroy_impl;

impl std::fmt::Debug for FleetSupervisorConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FleetSupervisorConfig")
            .field("fleet_key", &self.fleet_key)
            .finish_non_exhaustive()
    }
}

impl FleetSupervisor {
    /// Go SDK parity: a scale set must have labels; when the fleet
    /// declares none, default to a single System label carrying the
    /// scale-set name (upstream `ensureLabels`).
    pub fn fallback_labels(&self) -> Vec<shaula_core::github::Label> {
        if self.config.labels.is_empty() {
            return vec![
                shaula_core::github::Label::system(self.identity.scale_set_name.clone()).unwrap_or(
                    shaula_core::github::Label {
                        name: self.identity.scale_set_name.clone(),
                        label_type: "System".to_string(),
                    },
                ),
            ];
        }
        self.config.labels.clone()
    }
}

/// Constructor dependencies for [`FleetSupervisor::new`].
pub struct FleetSupervisorDeps {
    pub limits: LifecycleLimits,
    pub store: Arc<dyn LifecycleStore>,
    pub handoff: Arc<dyn shaula_core::registry::ControlPlaneStore>,
    pub github: Arc<dyn GitHubAccessPort>,
    pub runtime: Arc<dyn TemplateRuntimePort>,
}

#[derive(Clone)]
pub struct LifecycleLimits {
    pub create: Arc<tokio::sync::Semaphore>,
    pub destroy: Arc<tokio::sync::Semaphore>,
}
