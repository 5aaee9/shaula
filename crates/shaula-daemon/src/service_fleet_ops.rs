//! Fleet status and decommission implementations, split out of the trait
//! impl to keep each file within the 400-line limit (AGENTS.md).

use super::{unprocessable, AuthRevisionRef, ControlPlane};
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
        let capacity_policy = self
            .store
            .fleet_revision_latest(key)
            .await?
            .and_then(|r| serde_json::from_str::<FleetSpec>(&r.spec_json).ok())
            .map(|spec| shaula_core::capacity::CapacityPolicy::from(spec.capacity));

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
        let auth = handoff.map(|h| AuthRolloutSummary {
            desired: h.desired.clone(),
            observed: h.observed.clone(),
            handoff_state: h.state,
        });
        let auth_observed_matches = auth
            .as_ref()
            .map(|a| a.observed == Some(a.desired.clone()))
            .unwrap_or(false);
        let auth_lag_reason = auth.as_ref().and_then(|a| {
            if a.observed != Some(a.desired.clone()) {
                Some("AuthHandoffLag".to_string())
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
                        && fleet.phase == "Ready",
                    reason: None,
                },
            ],
            capacity: CapacitySummary {
                assigned_demand,
                target: capacity_target,
                effective,
                occupancy,
            },
            last_error: None,
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
                "fleet",
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
            resource_kind: "fleet",
            resource_key: key.to_string(),
            incarnation: fleet.incarnation.clone(),
            revision: new_revision,
            spec_json: "{}".to_string(),
            template: None,
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

impl ControlPlane {
    /// Resolves everything fleet PUT admission needs from the two
    /// authorities: the retained-or-resolved exact template pin, and the
    /// resolved active auth revision. Inputs are validated against the
    /// pinned revision's parameter schema AND its finite alias policy
    /// (spec 0002 section 4.1, 0005 section 5.1).
    pub(crate) async fn resolve_admission_materials(
        &self,
        key: &str,
        spec: &FleetSpec,
    ) -> CoreResult<Result<(Option<(String, i64, String, String)>, AuthRevisionRef), MutationError>>
    {
        let previous_row = self.store.fleet_revision_latest(key).await?;
        let previous_pin: Option<(String, i64, String, String)> =
            previous_row.as_ref().and_then(|r| {
                let (k, rev) = (r.template_profile_key.clone()?, r.template_revision?);
                Some((
                    k,
                    rev,
                    r.template_artifact_digest.clone()?,
                    r.template_attestation_id.clone()?,
                ))
            });
        let reference_unchanged = previous_row.as_ref().is_some_and(|prev| {
            let prev_spec: std::result::Result<FleetSpec, _> =
                serde_json::from_str(&prev.spec_json);
            match prev_spec {
                Ok(ps) => super::template_referenced(&ps) == super::template_referenced(spec),
                Err(_) => false,
            }
        });
        let template = if reference_unchanged {
            previous_pin.clone()
        } else {
            match self.resolve_template_ref(&spec.template_profile_ref).await {
                Ok(found) => Some(found),
                Err(e) => return Ok(Err(unprocessable(e.code, e.summary))),
            }
        };
        if let Some((pin_key, pin_rev, pin_artifact, _)) = &template {
            let policy = self
                .store
                .template_revision_get(pin_key, *pin_rev)
                .await?
                .and_then(|r| r.fleet_input_policy_json)
                .unwrap_or_else(|| "{}".into());
            // Layer 1 authority: the artifact's declared parameter schema
            // (required/type/bounds) from the exact pinned artifact. A
            // read failure is an Err (`?` → 500): admission never
            // degrades a corrupt artifact to "no schema" (F08). A blank
            // document is equally corrupt — an empty schema is `{}`, not
            // whitespace (R5-04).
            let schema = self.store.artifact_parameter_schema(pin_artifact).await?;
            if schema.trim().is_empty() {
                return Err(CoreError::new(
                    ReasonCode::StorageUnavailable,
                    "pinned artifact has a blank parameter schema document",
                ));
            }
            // A rejected input is a CLIENT error: classified 422, never a
            // 500 (spec 0002 section 5.1 invalid-spec contract).
            if let Err(e) = super::validate_inputs(&spec.template_inputs, &policy, Some(&schema)) {
                return Ok(Err(unprocessable(e.code, e.summary)));
            }
        }
        if let Err(e) = self
            .assert_auth_target_allowed(&spec.github.auth_profile_ref, spec)
            .await
        {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }
        let resolved_auth = match self.resolve_auth_ref(&spec.github.auth_profile_ref).await {
            Ok(found) => found,
            Err(e) => return Ok(Err(unprocessable(e.code, e.summary))),
        };
        Ok(Ok((template, resolved_auth)))
    }
}
