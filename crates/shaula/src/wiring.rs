//! Composition-root wiring for the daemon's core work loop (R9-01,
//! spec 0001 §11 / 0002 §10). The architecture forbids the daemon crate
//! from depending on the template/scaleset adapters, so the concrete
//! per-fleet supervisors are assembled HERE and driven on a reconcile
//! loop: every tick enumerates active fleets, binds a supervisor to the
//! fleet's CURRENT active credential revision, and runs one reconcile
//! pass (handoff ack, ownership create-or-adopt, capacity convergence,
//! safe retirement/destroy).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use shaula_core::error::CoreResult;
use shaula_core::ports::{Clock, TemplateRuntimePort};
use shaula_core::registry::{Actor, ControlPlaneStore, LifecycleStore, Scope};
use shaula_daemon::effect_gate::FleetEffectGates;
use shaula_daemon::supervisor::FleetSupervisor;

/// The daemon-side reconcile loop. Constructed by the composition root
/// with THE shared effect-gate arc so apply admission and capacity
/// reconciliation serialize on the same gates (R6-02).
pub struct SupervisorWiring {
    limits: shaula_daemon::supervisor::LifecycleLimits,
    store: Arc<dyn ControlPlaneStore>,
    lifecycle: Arc<dyn LifecycleStore>,
    runtime: Arc<dyn TemplateRuntimePort>,
    setup_info_issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
    clock: Arc<dyn Clock>,
    gates: Arc<FleetEffectGates>,
    work_root: PathBuf,
    artifact_root: PathBuf,
    operation_timeout: Duration,
    /// Cached supervisors keyed by (fleet, phase, desired revision, auth
    /// revision): a credential rotation, fleet replacement or phase
    /// change rebuilds the client.
    cache: HashMap<supervisor::SupervisorCacheKey, Arc<FleetSupervisor>>,
    listeners: HashMap<String, Arc<shaula_daemon::listener::FleetListener>>,
    tasks: crate::fleet_tasks::FleetTasks,
    /// Per-profile auth-worker deferral deadlines (F8): a rate-limited
    /// validation carries GitHub's Retry-After; the next attempt waits
    /// until the deadline instead of polling every scan interval.
    auth_worker_deferred_until: Arc<tokio::sync::Mutex<HashMap<(String, i64), i64>>>,
    /// Transport seam for the auth worker (G1): production wiring pins
    /// the fixed github.com endpoints; tests inject a scripted server.
    auth_worker_endpoints: crate::auth_worker_probe::WorkerEndpoints,
}

impl SupervisorWiring {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn ControlPlaneStore>,
        lifecycle: Arc<dyn LifecycleStore>,
        runtime: Arc<dyn TemplateRuntimePort>,
        clock: Arc<dyn Clock>,
        gates: Arc<FleetEffectGates>,
        work_root: PathBuf,
        artifact_root: PathBuf,
        operation_timeout: Duration,
        limits: shaula_daemon::supervisor::LifecycleLimits,
    ) -> Self {
        Self {
            limits,
            store,
            lifecycle,
            runtime,
            setup_info_issuer: None,
            clock,
            gates,
            work_root,
            artifact_root,
            operation_timeout,
            cache: HashMap::new(),
            listeners: HashMap::new(),
            tasks: crate::fleet_tasks::FleetTasks::default(),
            auth_worker_deferred_until: Arc::default(),
            auth_worker_endpoints: crate::auth_worker_probe::WorkerEndpoints::production(),
        }
    }

    pub fn with_setup_info_issuer(
        mut self,
        issuer: Option<Arc<dyn shaula_core::setup_info::SetupInfoIssuer>>,
    ) -> Self {
        self.setup_info_issuer = issuer;
        self
    }

    /// Injects auth-worker endpoints (composition/test seam; production
    /// keeps the fixed github.com configuration).
    #[cfg(test)]
    pub(crate) fn with_auth_worker_endpoints(
        mut self,
        endpoints: crate::auth_worker_probe::WorkerEndpoints,
    ) -> Self {
        self.auth_worker_endpoints = endpoints;
        self
    }

    /// Runs the reconcile loop until shutdown. Level-triggered like the
    /// scan loop: each pass is bounded, failures are logged per pass and
    /// the loop stays alive to retry.
    pub async fn run(mut self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(shaula_daemon::daemon::SCAN_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                joined = self.tasks.join_next(), if !self.tasks.is_empty() => {
                    if let Some((key, Err(e))) = joined {
                        tracing::warn!(fleet = %key, summary = %e, "fleet reconcile failed");
                    }
                }
                _ = ticker.tick() => {
                    let now = self.clock.now_unix_ms();
                    tokio::select! {
                        result = self.tick_all(now) => {
                            if let Err(e) = result {
                                tracing::warn!(summary = %e.summary, "supervisor reconcile pass failed");
                            }
                        }
                        _ = shutdown.changed() => break,
                    }
                }
                _ = shutdown.changed() => {
                    break;
                }
            }
        }
        self.tasks.shutdown().await;
    }

    async fn tick_all(&mut self, now: i64) -> CoreResult<()> {
        for key in self.store.auth_profile_keys().await? {
            // G1: the REAL Profile key reaches the worker — the namespaced
            // `auth/{key}` string is only the internal task identity. The
            // deferral gate is keyed by the CANDIDATE REF (profile +
            // desired revision, G2), so a new publication is never
            // stranded by an old revision's deadline.
            let Some(head) = self.store.auth_profile_get(&key).await? else {
                continue;
            };
            if head.status != "Validating" {
                continue;
            }
            let worker_key = format!("auth/{key}");
            if self.tasks.contains(&worker_key) {
                continue;
            }
            let gate_key = (key.clone(), head.desired_revision);
            if let Some(retry_at) = self.auth_worker_deferred_until.lock().await.get(&gate_key) {
                if now < *retry_at {
                    continue;
                }
            }
            // A new desired revision invalidates stale deferral state from
            // earlier revisions of this profile.
            self.auth_worker_deferred_until
                .lock()
                .await
                .retain(|(profile, revision), _| {
                    profile != &key || *revision == head.desired_revision
                });
            let deferred = self.auth_worker_deferred_until.clone();
            let store = self.store.clone();
            let clock = self.clock.clone();
            let endpoints = self.auth_worker_endpoints.clone();
            let real_key = key.clone();
            self.tasks.spawn(worker_key, async move {
                let flow = crate::auth_worker::validate(
                    store,
                    clock.clone(),
                    real_key,
                    gate_key.1,
                    &endpoints,
                )
                .await;
                let mut gate = deferred.lock().await;
                match &flow {
                    Ok(crate::auth_worker::WorkerFlow::Deferred { retry_at_unix_ms }) => {
                        // G2: None means bounded normal backoff — never an
                        // infinite deadline. Deadlines are ABSOLUTE unix
                        // ms, so the default backoff anchors at `now`.
                        gate.insert(
                            gate_key,
                            retry_at_unix_ms.unwrap_or_else(|| {
                                clock
                                    .now_unix_ms()
                                    .saturating_add(crate::auth_worker::DEFAULT_RETRY_BACKOFF_MS)
                            }),
                        );
                    }
                    Err(_) => {
                        gate.insert(
                            gate_key,
                            clock
                                .now_unix_ms()
                                .saturating_add(crate::auth_worker::DEFAULT_RETRY_BACKOFF_MS),
                        );
                    }
                    Ok(crate::auth_worker::WorkerFlow::Done) => {
                        gate.remove(&gate_key);
                    }
                }
                flow.map(|_| ())
            });
        }
        let mut live_keys = std::collections::HashSet::new();
        let actor = Actor {
            name: "shaula-daemon".to_string(),
            scopes: vec![
                Scope::FleetRead,
                Scope::FleetWrite,
                Scope::TemplateRead,
                Scope::AuthRead,
            ],
        };
        // The list excludes tombstoned fleets; deletion-marked fleets
        // still tick so their decommission cleanup converges (0002 §8).
        // R10-01: the desired revision is part of the supervisor cache
        // key — a capacity/labels/identity/template change mints a new
        // revision, so a stale-configured supervisor can never be
        // reused for reconcile.
        // fleet_list element 3 is the fleet PHASE (Pending/Decommissioning/...),
        // which also belongs in the cache key: a phase transition rebuilds
        // the supervisor alongside the revision-driven rebuild (R10-01).
        for (key, revision, phase) in self.store.fleet_list(&actor).await? {
            live_keys.insert(key.clone());
            match self.supervisor_for(&key, revision, &phase).await {
                Ok(Some(supervisor)) => {
                    let listener_key = format!("listener/{key}");
                    if !self.tasks.contains(&listener_key) {
                        if let Some(listener) = supervisor.listener() {
                            self.tasks
                                .spawn(listener_key, async move { listener.poll_once().await });
                        }
                    }
                    if !self.tasks.contains(&key) {
                        self.tasks
                            .spawn(key, async move { supervisor.tick(now).await.map(|_| ()) });
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(fleet = %key, summary = %e.summary, "fleet wiring failed"),
            }
        }
        // R10-01: evict supervisors for revisions that no longer exist —
        // the cache stays bounded by live fleet revisions only.
        self.cache.retain(|k, _| live_keys.contains(&k.fleet));
        self.listeners.retain(|key, _| live_keys.contains(key));
        Ok(())
    }
}

#[cfg(test)]
impl SupervisorWiring {
    /// Test-only drain: WAITS for all spawned worker/fleet tasks to
    /// complete (no aborts) so a scheduling test can observe their
    /// outcomes deterministically.
    pub(crate) async fn drain_for_tests(&mut self) {
        while !self.tasks.is_empty() {
            let _ = self.tasks.join_next().await;
        }
    }
}

/// Composition tests (real scheduling loop + handoff→execution authority)
/// live in `wiring_tests.rs`, declared as this module's child so they can
/// reach private internals like `tick_all` and `supervisor_for`.
#[cfg(test)]
#[path = "wiring_tests.rs"]
pub(crate) mod wiring_tests;

/// G9 end-to-end composition (HTTP publication → real scheduler → real
/// worker → HTTP correction), also a child module of `wiring`.
#[cfg(test)]
#[path = "wiring_http_tests.rs"]
pub(crate) mod http_tests;

#[path = "wiring_supervisor.rs"]
mod supervisor;
