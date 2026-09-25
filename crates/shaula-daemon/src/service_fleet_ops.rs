//! Fleet status and decommission implementations, split out of the trait
//! impl to keep each file within the 400-line limit (AGENTS.md).

use super::{unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetSpec;
use shaula_core::registry::{
    Actor, AuthRolloutSummary, CapacitySummary, ChangeView, Condition, DependencySummary,
    FleetStatus, MutationAccepted, MutationError, MutationFacts, Scope,
};

impl ControlPlane {
    pub(crate) async fn fleet_status_impl(
        &self,
        key: &str,
    ) -> CoreResult<Result<FleetStatus, MutationError>> {
        let Some(fleet) = self.store.fleet_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let handoff = self.store.handoff_get(key).await?;
        let demand = self.store.demand_get(key).await?;
        let (effective, occupancy) = self.store.capacity_counters(key).await?;
        // Read the pinned capacity policy from the latest revision so the
        // status target uses the contract formula min(max, min+demand).
        let latest = self.store.fleet_revision_latest(key).await?;
        let spec = latest
            .as_ref()
            .and_then(|r| serde_json::from_str::<FleetSpec>(&r.spec_json).ok());
        let forgejo = spec
            .as_ref()
            .is_some_and(|spec| spec.kind == shaula_core::fleet::FleetProviderKind::Forgejo);
        let capacity_policy =
            spec.map(|spec| shaula_core::capacity::CapacityPolicy::from(spec.capacity));

        let assigned_demand = demand.unwrap_or(0);
        let capacity_target = match &capacity_policy {
            Some(policy) => shaula_core::capacity::target(
                policy,
                &shaula_core::capacity::AssignedDemand {
                    total_assigned_jobs: assigned_demand,
                },
            ),
            // Unknown spec (mid-admission): occupancy is the conservative
            // observable target.
            None => occupancy,
        };
        let auth_context = self.store.fleet_auth_context_get(key).await?.map(|c| {
            let route = |json: &Option<String>, reference: &Option<(String, i64)>| {
                let context: shaula_core::auth_context::ResolvedAuthContext = serde_json::from_str(json.as_deref()?).ok()?;
                if reference.as_ref() != Some(&(context.profile_key.clone(), context.revision)) {
                    return None;
                }
                Some(serde_json::json!({
                    "profileKey": context.profile_key, "revision": context.revision,
                    "githubHost": context.github_host, "appId": context.app_id,
                    "account": { "login": context.login, "id": context.account_id, "kind": context.account_kind },
                    "installationId": context.installation_id, "target": context.target,
                    "organizationId": context.organization_id, "repositoryId": context.repository_id,
                    "repositoryOwnerId": context.repository_owner_id,
                }))
            };
            let desired_route = route(&c.desired_context_json, &c.desired);
            let observed_route = route(&c.observed_context_json, &c.observed);
            let route_missing = c.desired.is_some() && desired_route.is_none()
                || c.observed.is_some() && observed_route.is_none();
            shaula_core::registry::AuthContextSummary {
                desired: c.desired,
                observed: c.observed,
                state: if route_missing && c.state != "Blocked" { "Unknown".into() } else { c.state },
                reason: c.reason.or_else(|| route_missing.then(|| "AuthContextUnavailable".into())),
                desired_route,
                observed_route,
            }
        });
        let auth = if forgejo {
            latest.map(|row| AuthRolloutSummary {
                observed: (fleet.phase == "Ready"
                    && fleet.observed_revision == fleet.desired_revision)
                    .then(|| row.auth_desired.clone()),
                desired: row.auth_desired,
                handoff_state: "NotApplicable".into(),
                context: None,
            })
        } else {
            handoff.map(|h| AuthRolloutSummary {
                desired: h.desired.clone(),
                observed: h.observed.clone(),
                handoff_state: h.state,
                context: auth_context,
            })
        };
        let auth_observed_matches = auth
            .as_ref()
            .map(|a| a.observed == Some(a.desired.clone()))
            .unwrap_or(false);
        let auth_lag_reason = auth.as_ref().and_then(|a| {
            if a.observed != Some(a.desired.clone()) {
                Some(
                    if forgejo {
                        "RunnerRebuildPending"
                    } else {
                        "AuthHandoffLag"
                    }
                    .to_string(),
                )
            } else {
                None
            }
        });

        Ok(Ok(FleetStatus {
            fleet_key: key.to_string(),
            desired_revision: fleet.desired_revision,
            observed_revision: fleet.observed_revision,
            phase: fleet.phase.clone(),
            dependencies: DependencySummary {
                template: None,
                auth,
            },
            conditions: vec![
                Condition {
                    condition_type: "SpecAccepted",
                    status: true,
                    reason: None,
                },
                Condition {
                    condition_type: "AuthRevisionObserved",
                    status: auth_observed_matches,
                    reason: auth_lag_reason,
                },
                Condition {
                    condition_type: "Converged",
                    status: fleet.observed_revision == fleet.desired_revision
                        && fleet.phase == "Ready"
                        && capacity_policy.is_some()
                        && effective == capacity_target
                        && occupancy == capacity_target,
                    reason: fleet.last_condition_reason.clone(),
                },
            ],
            capacity: CapacitySummary {
                assigned_demand,
                target: capacity_target,
                effective,
                occupancy,
            },
            last_error: fleet.last_condition_reason,
        }))
    }

    pub(crate) async fn fleet_delete_impl(
        &self,
        actor: &Actor,
        key: &str,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::FleetRetire) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing fleet.retire scope",
            )));
        }
        let precondition = match &if_match {
            Some((incarnation, revision)) => format!("if-match:{incarnation}:{revision}"),
            None => "none".to_string(),
        };
        match self
            .idempotency_replay(
                actor,
                ("fleet", "v1:DELETE"),
                key,
                &idempotency_key,
                "decommission",
                &precondition,
            )
            .await?
        {
            Err(conflict) => return Ok(Err(conflict)),
            Ok(Some(accepted)) => return Ok(Ok(accepted)),
            Ok(None) => {}
        }
        let Some((incarnation, revision)) = if_match else {
            return Ok(Err(MutationError::PreconditionRequired));
        };
        let Some(fleet) = self.store.fleet_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if fleet.incarnation != incarnation || fleet.desired_revision != revision {
            return Ok(Err(MutationError::PreconditionFailed {
                current: (fleet.incarnation.clone(), fleet.desired_revision),
            }));
        }
        if fleet.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let now = self.now_ms();
        let change_id = self.new_id();
        let new_revision = fleet.desired_revision + 1;

        let accepted = MutationAccepted {
            etag: format!("{}:{}", fleet.incarnation, new_revision),
            change: ChangeView {
                id: change_id.clone(),
                resource_kind: "fleet".to_string(),
                resource_key: key.to_string(),
                revision: new_revision,
                kind: "Decommission".to_string(),
                state: "Pending".to_string(),
                reason: None,
            },
            no_op: false,
        };

        // Exact DELETE retries replay the original result (0002 §5.2).
        let idempotency = idempotency_key.map(|idem| {
            let request_hash =
                self.idempotency_hash("fleet", key, &idem, "decommission", &precondition);
            let response_body = serde_json::to_string(&accepted)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
            Ok((idem, request_hash, 202, response_body))
        });
        let idempotency = match idempotency {
            Some(Ok(value)) => Some(value),
            Some(Err(e)) => return Err(e),
            None => None,
        };

        let facts = MutationFacts {
            idempotency_operation: "v1:DELETE",
            authentication: actor.authentication.clone(),
            resource_kind: "fleet",
            resource_key: key.to_string(),
            incarnation: fleet.incarnation.clone(),
            revision: new_revision,
            spec_json: "{}".to_string(),
            template: None,
            template_pool: Vec::new(),
            template_pool_ref: None,
            auth_desired: None,
            inputs_digest: "decommission".to_string(),
            actor: actor.name.clone(),
            now,
            change: accepted.change.clone(),
            outbox_topic: "fleet.change".to_string(),
            outbox_payload: format!("{{\"change\":\"{change_id}\"}}"),
            idempotency,
        };
        // R5-02: the deletion commit takes the fleet's effect gate
        // EXCLUSIVELY — it waits for every in-flight apply admission
        // claim to be released (apply terminated) and blocks new claims
        // while it commits. An apply admitted afterwards fails the CAS's
        // in-transaction head fence, so nothing spawns against a
        // decommissioned fleet.
        let effect_gate = self.effect_gates.acquire_exclusive(key).await;
        let committed = self.store.commit_decommission(facts).await;
        drop(effect_gate);
        if let Err(fence) = committed? {
            return Ok(Err(fence));
        }
        Ok(Ok(accepted))
    }
}
