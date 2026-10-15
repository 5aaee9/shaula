//! Per-fleet supervision: ownership reconciliation, capacity convergence
//! and the provider-neutral runner lifecycle. One tick is one pass of the
//! level-triggered reconcile; every external effect is preceded by a
//! durable intent and followed by a durable result.

use std::sync::Arc;

use sha2::Digest;
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
    destroy_backoff: crate::runner_operation::DestroyBackoff,
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

#[path = "supervisor_reconcile.rs"]
mod reconcile_impl;
