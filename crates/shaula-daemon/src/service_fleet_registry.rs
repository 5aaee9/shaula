//! Fleet Registry port implementation on the control-plane service.

use async_trait::async_trait;

use super::{template_referenced, unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::TemplateProfileRefDto;
use shaula_core::fleet::{normalize_fleet, validate_fleet_spec, FleetSpec};
use shaula_core::registry::HealthPort;
use shaula_core::registry::{
    Actor, ChangeView, FleetRegistryPort, FleetResource, FleetStatus, MutationAccepted,
    MutationError, MutationFacts, Scope,
};

#[async_trait]
impl HealthPort for ControlPlane {
    async fn live(&self) -> bool {
        true
    }

    async fn ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl FleetRegistryPort for ControlPlane {
    async fn fleet_put(
        &self,
        actor: &Actor,
        key: &str,
        spec: FleetSpec,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::FleetWrite) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing fleet.write scope",
            )));
        }
        if let Err(e) = validate_fleet_spec(&spec) {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }

        // Canonical request facts for idempotency: body + precondition.
        let canonical_body = serde_json::to_string(&spec)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        let precondition = match (&if_match, if_none_match) {
            (Some((incarnation, revision)), _) => format!("if-match:{incarnation}:{revision}"),
            (None, true) => "if-none-match:*".to_string(),
            (None, false) => "none".to_string(),
        };

        match self
            .idempotency_replay(
                "fleet",
                key,
                &idempotency_key,
                &canonical_body,
                &precondition,
            )
            .await?
        {
            Err(conflict) => return Ok(Err(conflict)),
            Ok(Some(accepted)) => return Ok(Ok(accepted)),
            Ok(None) => {}
        }

        let existing = self.store.fleet_get(key).await?;
        // A decommissioning fleet rejects every new revision with 410
        // regardless of preconditions (0002 section 8: DELETE is irreversible).
        if let Some(current) = &existing {
            if current.deletion_marker || current.tombstone {
                return Ok(Err(MutationError::Gone {
                    tombstone: key.to_string(),
                }));
            }
        }
        match (&existing, if_none_match, &if_match) {
            (None, true, _) => {}
            (Some(current), true, _) => {
                return Ok(Err(MutationError::PreconditionFailed {
                    current: (current.incarnation.clone(), current.desired_revision),
                }));
            }
            (None, false, _) => return Ok(Err(MutationError::PreconditionRequired)),
            (Some(current), false, Some((incarnation, revision))) => {
                if current.incarnation != *incarnation || current.desired_revision != *revision {
                    return Ok(Err(MutationError::PreconditionFailed {
                        current: (current.incarnation.clone(), current.desired_revision),
                    }));
                }
                if current.tombstone {
                    return Ok(Err(MutationError::Gone {
                        tombstone: key.to_string(),
                    }));
                }
            }
            (Some(_), false, None) => return Ok(Err(MutationError::PreconditionRequired)),
        }

        // Fleet key count admission limit.
        if existing.is_none() {
            let active = self.store.fleet_count().await?;
            if active >= self.max_active_fleets {
                return Ok(Err(MutationError::TooManyRequests {
                    retry_after_secs: 5,
                }));
            }
        }

        // Admission: an already-admitted Fleet keeps its retained exact pin
        // when the reference is unchanged; only a NEW key/revision resolves
        // current Active (and then requires zero occupancy) — spec 0005
        // section 5.1/158. Inputs are validated against both authorities
        // (schema + alias policy). Extracted to `service_fleet_ops`.
        let (template, resolved_auth) = match self.resolve_admission_materials(key, &spec).await? {
            Ok(materials) => materials,
            Err(e) => return Ok(Err(e)),
        };

        // Replacement gates: remote identity immutable per incarnation;
        // template/auth-key replacement requires zero occupancy.
        if let Some(_current) = &existing {
            let Some(previous) = self.store.fleet_revision_latest(key).await? else {
                return Ok(Err(MutationError::NotFound));
            };
            let previous_spec: FleetSpec = serde_json::from_str(&previous.spec_json)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
            if previous_spec.github.target != spec.github.target
                || previous_spec.github.scale_set_name != spec.github.scale_set_name
                || previous_spec.github.runner_group != spec.github.runner_group
                || previous_spec.github.labels != spec.github.labels
            {
                return Ok(Err(MutationError::IdentityConflict));
            }
            if template_referenced(&previous_spec) != template_referenced(&spec) {
                let occupancy = self.store.generations_occupancy(key).await?;
                if occupancy > 0 {
                    return Ok(Err(MutationError::RetirementBlocked {
                        reason: "replacement requires zero resource occupancy".into(),
                    }));
                }
            }
            if previous.auth_desired.0 != spec.github.auth_profile_ref {
                let occupancy = self.store.generations_occupancy(key).await?;
                if occupancy > 0 {
                    return Ok(Err(MutationError::RetirementBlocked {
                        reason: "auth profile replacement requires zero resource occupancy".into(),
                    }));
                }
            }
        }

        // No-op detection: same canonical spec AND same resolved pin is an
        // identical re-assertion — a durable 200 with NO new
        // Revision/Change (spec 0002 section 5.2/5.3). Same-Profile auth
        // promotion is not part of the comparison (section 4.2); rotation
        // propagates via the auth handoff retarget instead. The no-op is
        // itself durable: the audit entry and, with an idempotency key,
        // the 200 replay body are persisted, so a lost response replays
        // and a key reuse with a different body conflicts.
        if let Some((incarnation, revision)) = self
            .detect_noop(key, existing.as_ref(), &spec, template.as_ref())
            .await?
        {
            let accepted = MutationAccepted {
                etag: format!("{incarnation}:{revision}"),
                change: ChangeView {
                    id: String::new(),
                    resource_kind: "fleet".into(),
                    resource_key: key.to_string(),
                    revision,
                    kind: "NoOp".into(),
                    state: "NoOp".into(),
                    reason: None,
                },
                no_op: true,
            };
            let idempotency = match idempotency_key.map(|idem| {
                let request_hash =
                    self.idempotency_hash("fleet", key, &idem, &canonical_body, &precondition);
                let response_body = serde_json::to_string(&accepted)
                    .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
                Ok(shaula_core::registry::IdempotencyInsert {
                    id: format!("idem-noop-{}", self.new_id()),
                    resource_kind: "fleet".to_string(),
                    resource_key: key.to_string(),
                    idempotency_key: idem,
                    request_hash,
                    response_status: 200,
                    response_body: Some(response_body),
                    now: self.now_ms(),
                })
            }) {
                Some(Ok(value)) => Some(value),
                Some(Err(e)) => return Err(e),
                None => None,
            };
            let commit_result = self
                .store
                .commit_fleet_noop(
                    key,
                    &incarnation,
                    revision,
                    &actor.name,
                    idempotency,
                    self.now_ms(),
                )
                .await?;
            // A concurrent newer PUT or DELETE between classification and
            // commit invalidates the no-op: surface the precondition, never
            // record a stale 200 (spec 0002 section 5.3).
            match commit_result {
                Ok(()) => return Ok(Ok(accepted)),
                Err(fence) => return Ok(Err(fence)),
            }
        }

        let normalized = normalize_fleet(&spec, self.inputs_digest(&spec))?;
        let spec_json = serde_json::to_string(&spec)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        let revision = existing
            .as_ref()
            .map(|f| f.desired_revision + 1)
            .unwrap_or(1);
        let incarnation = existing
            .as_ref()
            .map(|f| f.incarnation.clone())
            .unwrap_or_else(|| self.new_id());
        let now = self.now_ms();
        let change_id = self.new_id();
        let kind = if existing.is_some() {
            "Replace"
        } else {
            "Create"
        };

        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{revision}"),
            change: ChangeView {
                id: change_id.clone(),
                resource_kind: "fleet".to_string(),
                resource_key: key.to_string(),
                revision,
                kind: kind.to_string(),
                state: "Pending".to_string(),
                reason: None,
            },
            no_op: false,
        };

        // The idempotency record stores the full serialized response so a
        // lost-response retry replays the exact original result (0002 section 5.2).
        let idempotency = idempotency_key.map(|idem| {
            let request_hash =
                self.idempotency_hash("fleet", key, &idem, &canonical_body, &precondition);
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
            incarnation: incarnation.clone(),
            revision,
            spec_json,
            template,
            auth_desired: Some((
                resolved_auth.profile_key.as_str().to_string(),
                resolved_auth.revision as i64,
            )),
            inputs_digest: normalized.inputs_digest,
            actor: actor.name.clone(),
            now,
            change: accepted.change.clone(),
            outbox_topic: "fleet.change".to_string(),
            outbox_payload: format!("{{\"change\":\"{change_id}\"}}"),
            idempotency,
        };
        // A lost fence race surfaces as a precondition failure so the
        // client re-reads the current desired head; never overwrite.
        // R6-02: the head-advancing commit takes the fleet's effect
        // gate EXCLUSIVELY — a Create holding its short admission claim
        // (durable ApplyStarting → spawn handover) commits BEFORE this
        // PUT, so it can never spawn against the NEW desired revision;
        // once the claim is released at the spawn handover, the PUT
        // commits freely and the next reconcile tick converges.
        let effect_gate = self.effect_gates.acquire_exclusive(key).await;
        let committed = self.store.commit_fleet_mutation(facts).await;
        drop(effect_gate);
        if let Err(fence) = committed? {
            return Ok(Err(fence));
        }
        Ok(Ok(accepted))
    }

    async fn fleet_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<FleetResource, MutationError>> {
        let Some(fleet) = self.store.fleet_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if fleet.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let Some(revision) = self.store.fleet_revision_latest(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let spec: FleetSpec = serde_json::from_str(&revision.spec_json)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        Ok(Ok(FleetResource {
            key: key.to_string(),
            spec,
            incarnation: fleet.incarnation,
            revision: fleet.desired_revision,
            resolved_template: revision.template_profile_key.clone().map(|k| {
                (
                    k,
                    revision.template_revision.unwrap_or_default(),
                    revision
                        .template_artifact_digest
                        .clone()
                        .unwrap_or_default(),
                    revision.template_attestation_id.clone().unwrap_or_default(),
                )
            }),
            resolved_auth: revision.auth_desired.clone(),
            created_at: revision.created_at,
            updated_at: revision.created_at,
        }))
    }

    async fn fleet_status_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<FleetStatus, MutationError>> {
        self.fleet_status_impl(key).await
    }

    async fn fleet_list(&self, _actor: &Actor) -> CoreResult<Vec<(String, i64, String)>> {
        self.store.fleet_list(_actor).await
    }

    async fn fleet_delete(
        &self,
        actor: &Actor,
        key: &str,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.fleet_delete_impl(actor, key, if_match, idempotency_key)
            .await
    }

    async fn fleet_change_get(
        &self,
        _actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
        self.store.fleet_change_get(change_id).await
    }

    async fn resolve_template_ref(
        &self,
        reference: &TemplateProfileRefDto,
    ) -> CoreResult<Option<(String, i64, String, String)>> {
        ControlPlane::resolve_template_ref(self, reference)
            .await
            .map(Some)
    }
}
