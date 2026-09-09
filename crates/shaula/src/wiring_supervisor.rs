//! Fleet client construction from exact desired and observed auth authority.

use super::SupervisorWiring;
use shaula_core::error::CoreResult;
use shaula_core::github::ScaleSetIdentity;
use shaula_core::ports::GitHubAccessPort;
use shaula_daemon::apply_intent::LedgerApplyIntentSink;
use shaula_daemon::supervisor::{FleetSupervisor, FleetSupervisorConfig, FleetSupervisorDeps};
use std::sync::Arc;

#[derive(Hash, PartialEq, Eq, Clone)]
pub(super) struct SupervisorCacheKey {
    pub fleet: String,
    phase: String,
    fleet_revision: i64,
    fence: i64,
    desired: (String, i64),
    observed: Option<(String, i64)>,
    desired_context: Option<String>,
    observed_context: Option<String>,
    execution_contexts: Vec<(String, i64, Option<String>)>,
}

#[cfg(test)]
#[path = "wiring_unsupported_auth_tests.rs"]
mod unsupported_auth_tests;

impl SupervisorWiring {
    pub(super) async fn supervisor_for(
        &mut self,
        key: &str,
        revision: i64,
        phase: &str,
    ) -> CoreResult<Option<Arc<FleetSupervisor>>> {
        let Some(latest) = self.store.fleet_revision_latest(key).await? else {
            return Ok(None);
        };
        if latest.revision != revision {
            return Ok(None);
        }
        let Ok(spec) = serde_json::from_str::<shaula_core::fleet::FleetSpec>(&latest.spec_json)
        else {
            return Ok(None);
        };
        // G3: explicit DESIRED vs OBSERVED authority. The EXECUTION
        // revision is the retained observed reference while a handoff is
        // in flight; otherwise the active head. Effects run only through
        // a port bound to the matching persisted context; corrupt
        // observed JSON fails closed (no executable port).
        let handoff = self.store.handoff_get(key).await?;
        let context_row = self.store.fleet_auth_context_get(key).await?;
        let desired = handoff
            .as_ref()
            .map(|h| h.desired.clone())
            .unwrap_or_else(|| latest.auth_desired.clone());
        let observed = handoff.as_ref().and_then(|h| h.observed.clone());
        let (exec_profile, exec_revision) = observed.clone().unwrap_or_else(|| desired.clone());
        let exec_row = self
            .store
            .auth_revision_get(&exec_profile, exec_revision)
            .await?;
        if exec_row
            .as_ref()
            .is_none_or(|r| r.schema_version != 2 || r.kind != "github_app")
        {
            return Ok(None);
        }
        let mut observed_context = None;
        if let Some(row) = &context_row {
            if row.observed.as_ref() == Some(&(exec_profile.clone(), exec_revision)) {
                if let Some(json) = &row.observed_context_json {
                    match serde_json::from_str::<shaula_core::auth_context::ResolvedAuthContext>(
                        json,
                    ) {
                        Ok(ctx) => observed_context = Some(ctx),
                        // A corrupt observed authority for a v2 execution
                        // ref can never be replaced by an in-memory pin.
                        Err(_) => return Ok(None),
                    }
                }
            }
        }
        if observed.is_some() {
            let Some(ctx) = &observed_context else {
                return Ok(None);
            };
            if ctx.profile_key != exec_profile
                || ctx.revision != exec_revision
                || ctx.target != spec.github.target
                || !ctx.has_complete_identity()
            {
                return Ok(None);
            }
        }
        let execution_ready = observed.is_some();
        let fence = self
            .store
            .fleet_get(key)
            .await?
            .map(|f| f.mutation_fence)
            .unwrap_or_default();
        let mut execution_contexts = Vec::new();
        for (profile, revision) in self.store.auth_execution_refs(key).await? {
            let context = self
                .store
                .auth_execution_context_get(key, &profile, revision)
                .await?;
            let json = context
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| {
                    shaula_core::error::CoreError::new(
                        shaula_core::error::ReasonCode::Internal,
                        "execution context serialization failed",
                    )
                })?;
            execution_contexts.push((profile, revision, json));
        }
        execution_contexts.sort();
        let cache_key = SupervisorCacheKey {
            fleet: key.to_string(),
            phase: phase.to_string(),
            fleet_revision: revision,
            fence,
            desired: desired.clone(),
            observed,
            desired_context: context_row
                .as_ref()
                .and_then(|r| r.desired_context_json.clone()),
            observed_context: context_row
                .as_ref()
                .and_then(|r| r.observed_context_json.clone()),
            execution_contexts,
        };
        if !self.cache.contains_key(&cache_key) {
            let Some(credential) = crate::wiring_credential::build_credential(
                &self.store,
                &exec_profile,
                exec_revision,
                &spec.github.target,
            )
            .await?
            else {
                return Ok(None);
            };
            let client = self
                .auth_worker_endpoints
                .probe_client(spec.github.target.clone(), credential, self.clock.clone())?
                .with_expected_context(observed_context);
            let github: Arc<dyn GitHubAccessPort> = Arc::new(client);
            let mut revision_clients = std::collections::HashMap::new();
            for (profile, revision, json) in &cache_key.execution_contexts {
                if profile == &exec_profile && *revision == exec_revision && execution_ready {
                    revision_clients.insert((profile.clone(), *revision), github.clone());
                    continue;
                }
                let row = self.store.auth_revision_get(profile, *revision).await?;
                if row.as_ref().is_none_or(|r| {
                    r.schema_version != 2 || r.kind != "github_app" || json.is_none()
                }) {
                    continue; // no authority: cleanup retains occupancy and retries
                }
                let context = json
                    .as_ref()
                    .map(|value| {
                        serde_json::from_str::<shaula_core::auth_context::ResolvedAuthContext>(
                            value,
                        )
                    })
                    .transpose()
                    .map_err(|_| {
                        shaula_core::error::CoreError::new(
                            shaula_core::error::ReasonCode::Internal,
                            "retained execution context invalid",
                        )
                    })?;
                if context.as_ref().is_none_or(|c| {
                    c.profile_key != *profile
                        || c.revision != *revision
                        || c.target != spec.github.target
                        || !c.has_complete_identity()
                }) {
                    continue;
                }
                let Some(credential) = crate::wiring_credential::build_credential(
                    &self.store,
                    profile,
                    *revision,
                    &spec.github.target,
                )
                .await?
                else {
                    continue;
                };
                let client = self
                    .auth_worker_endpoints
                    .probe_client(spec.github.target.clone(), credential, self.clock.clone())?
                    .with_expected_context(context);
                revision_clients.insert(
                    (profile.clone(), *revision),
                    Arc::new(client) as Arc<dyn GitHubAccessPort>,
                );
            }
            // A rotation is verified with its desired credential. Existing
            // effects remain bound to their observed credential/context.
            let Some(desired_credential) = crate::wiring_credential::build_credential(
                &self.store,
                &desired.0,
                desired.1,
                &spec.github.target,
            )
            .await?
            else {
                return Ok(None);
            };
            let handoff_github = Arc::new(self.auth_worker_endpoints.probe_client(
                spec.github.target.clone(),
                desired_credential,
                self.clock.clone(),
            )?);
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
                    auth_profile_key: exec_profile.clone(),
                    auth_revision: exec_revision,
                },
                identity,
            )
            .with_handoff_github(handoff_github)
            .with_clock(self.clock.clone())
            .with_handoff_authority(desired)
            .with_execution_ready(execution_ready)
            .with_revision_clients(revision_clients);
            self.cache.retain(|k, _| k.fleet != key);
            self.cache.insert(cache_key.clone(), Arc::new(supervisor));
        }
        let supervisor = self.cache[&cache_key].clone();
        Ok(Some(supervisor))
    }
}
