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
use shaula_core::github::ScaleSetIdentity;
use shaula_core::ports::{Clock, GitHubAccessPort, TemplateRuntimePort};
use shaula_core::registry::{Actor, ControlPlaneStore, LifecycleStore, Scope};
use shaula_core::secret::SecretString;
use shaula_daemon::apply_intent::LedgerApplyIntentSink;
use shaula_daemon::effect_gate::FleetEffectGates;
use shaula_daemon::supervisor::{FleetSupervisor, FleetSupervisorConfig, FleetSupervisorDeps};
use shaula_scaleset::ScalesetClient;

/// The daemon-side reconcile loop. Constructed by the composition root
/// with THE shared effect-gate arc so apply admission and capacity
/// reconciliation serialize on the same gates (R6-02).
pub struct SupervisorWiring {
    limits: shaula_daemon::supervisor::LifecycleLimits,
    store: Arc<dyn ControlPlaneStore>,
    lifecycle: Arc<dyn LifecycleStore>,
    runtime: Arc<dyn TemplateRuntimePort>,
    clock: Arc<dyn Clock>,
    gates: Arc<FleetEffectGates>,
    work_root: PathBuf,
    artifact_root: PathBuf,
    operation_timeout: Duration,
    /// Cached supervisors keyed by (fleet, phase, desired revision, auth
    /// revision): a credential rotation, fleet replacement or phase
    /// change rebuilds the client.
    cache: HashMap<(String, String, i64, i64), Arc<FleetSupervisor>>,
    tasks: crate::fleet_tasks::FleetTasks,
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
            clock,
            gates,
            work_root,
            artifact_root,
            operation_timeout,
            cache: HashMap::new(),
            tasks: crate::fleet_tasks::FleetTasks::default(),
        }
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
            let worker_key = format!("auth/{key}");
            if !self.tasks.contains(&worker_key) {
                self.tasks.spawn(
                    worker_key,
                    crate::auth_worker::validate(self.store.clone(), self.clock.clone(), key),
                );
            }
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
            if self.tasks.contains(&key) {
                continue;
            }
            match self.supervisor_for(&key, revision, &phase).await {
                Ok(Some(supervisor)) => {
                    self.tasks
                        .spawn(key, async move { supervisor.tick(now).await.map(|_| ()) });
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(fleet = %key, summary = %e.summary, "fleet wiring failed"),
            }
        }
        // R10-01: evict supervisors for revisions that no longer exist —
        // the cache stays bounded by live fleet revisions only.
        self.cache.retain(|k, _| live_keys.contains(&k.0));
        Ok(())
    }
    async fn supervisor_for(
        &mut self,
        key: &str,
        revision: i64,
        phase: &str,
    ) -> CoreResult<Option<Arc<FleetSupervisor>>> {
        let Some(latest) = self.store.fleet_revision_latest(key).await? else {
            return Ok(None);
        };
        let Ok(spec) = serde_json::from_str::<shaula_core::fleet::FleetSpec>(&latest.spec_json)
        else {
            return Ok(None);
        };
        // The CURRENT active credential revision decides the client —
        // a rotation rebinds the supervisor to the new credential.
        let Some(auth) = self
            .store
            .auth_revision_active(&spec.github.auth_profile_ref)
            .await?
        else {
            return Ok(None);
        };
        let auth_profile_key = auth.profile_key.to_string();
        let auth_revision = auth.revision;
        let cache_key = (key.to_string(), phase.to_string(), revision, auth.revision);
        if !self.cache.contains_key(&cache_key) {
            let Some(credential) =
                build_credential(&self.store, &auth.profile_key, auth.revision).await?
            else {
                return Ok(None);
            };
            let Ok(client) = ScalesetClient::production(
                spec.github.target.clone(),
                credential,
                self.clock.clone(),
            ) else {
                return Ok(None);
            };
            let github: Arc<dyn GitHubAccessPort> = Arc::new(client);
            let identity = ScaleSetIdentity {
                target: spec.github.target.clone(),
                runner_group: spec.github.runner_group.clone(),
                scale_set_name: spec.github.scale_set_name.clone(),
            };
            let labels = spec
                .github
                .labels
                .iter()
                .map(|name| shaula_core::github::Label {
                    name: name.clone(),
                    label_type: "Customer".to_string(),
                })
                .collect();
            let supervisor = FleetSupervisor::new(
                FleetSupervisorDeps {
                    limits: self.limits.clone(),
                    store: self.lifecycle.clone(),
                    handoff: self.store.clone(),
                    github,
                    runtime: self.runtime.clone(),
                },
                FleetSupervisorConfig {
                    fleet_key: key.to_string(),
                    capacity: shaula_core::capacity::CapacityPolicy {
                        min_runners: spec.capacity.min_runners,
                        max_runners: spec.capacity.max_runners,
                    },
                    work_root: self.work_root.clone(),
                    operation_timeout: self.operation_timeout,
                    artifact_root: self.artifact_root.clone(),
                    apply_intent_sink: Arc::new(LedgerApplyIntentSink {
                        store: self.lifecycle.clone(),
                        gates: self.gates.clone(),
                    }),
                    labels,
                    auth_profile_key: auth_profile_key.clone(),
                    auth_revision,
                },
                identity,
            );
            self.cache.retain(|k, _| k.0 != key);
            self.cache.insert(cache_key.clone(), Arc::new(supervisor));
        }
        let supervisor = self.cache[&cache_key].clone();
        Ok(Some(supervisor))
    }
}

/// Loads the exact accepted credential for the ACTIVE auth revision
/// (bytes never leave the store seam until this protected handoff).
pub(super) async fn build_credential(
    store: &Arc<dyn ControlPlaneStore>,
    profile_key: &str,
    revision: i64,
) -> CoreResult<Option<shaula_scaleset::Credential>> {
    let Some(bytes) = store.auth_credential_bytes(profile_key, revision).await? else {
        return Ok(None);
    };
    let Some(row) = store.auth_revision_get(profile_key, revision).await? else {
        return Ok(None);
    };
    let secret = SecretString::new(String::from_utf8_lossy(&bytes).into_owned());
    let credential = match row.kind.as_str() {
        "pat" => shaula_scaleset::Credential::Pat(secret),
        "github_app" => shaula_scaleset::Credential::GitHubApp {
            client_id: row.app_id.clone().unwrap_or_default(),
            installation_id: row.installation_id.unwrap_or_default(),
            private_key: secret,
        },
        _ => return Ok(None),
    };
    Ok(Some(credential))
}
