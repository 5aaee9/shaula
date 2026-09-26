use super::*;
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

/// Static per-fleet configuration frozen at supervisor construction.
#[derive(Clone)]
pub struct FleetSupervisorConfig {
    pub fleet_key: String,
    pub capacity: shaula_core::capacity::CapacityPolicy,
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
    /// G3: the handoff settled THIS tick — no effects may run until the
    /// wiring re-reads the acknowledged context (next pass).
    pub settled_this_tick: bool,
    pub scale_set_bound: bool,
    pub created: u32,
    pub destroyed: u32,
    pub quarantined: u32,
    pub blocked: bool,
    pub reason: Option<shaula_core::error::ReasonCode>,
    pub listener_ready: bool,
    /// Spec 0002 §4.4: a deletion-marked fleet completes its Decommission
    /// Change only when every owned Generation is terminal. Set on the
    /// tick where the last non-terminal generation is proven gone.
    pub decommission_complete: bool,
    pub session_epoch: Option<i64>,
}
