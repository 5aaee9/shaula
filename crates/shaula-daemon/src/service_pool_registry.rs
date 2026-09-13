//! Shared TemplatePool Registry port implementation (spec 0037). Pools
//! ride the template permission family: publish replaces, read lists,
//! retire tombstones. Pool PUT admission resolves every member's bare
//! `template_profile_ref` to the profile's CURRENT Active revision — the
//! same follow-latest authority the inline pool path uses — and the
//! commit freezes those pins on immutable `(pool_key, pool_revision)`
//! member rows. Unlike a Fleet PUT, a pool admission is always an
//! explicit upgrade channel: the cascade would re-resolve to the same
//! Active revisions anyway, so an identical re-assertion whose members
//! resolved unchanged is the only no-op shape (spec 0037 §3).

use async_trait::async_trait;
use sha2::Digest;

use super::{unprocessable, ControlPlane};
use shaula_core::auth::is_stable_identifier;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, ChangeView, MutationAccepted, MutationError, MutationFacts, Scope,
    TemplatePoolRegistryPort, TemplatePoolResource,
};
use shaula_core::template_pool::{ResolvedTemplatePoolMember, TemplatePoolSpec};

impl ControlPlane {
    /// Resolves one pool spec's members to their profiles' current Active
    /// revisions, validating each member's inputs against the pinned
    /// revision's alias policy and artifact parameter schema — the same
    /// two-authority gate the inline pool path enforces (spec 0037 §2).
    pub(crate) async fn resolve_pool_members(
        &self,
        spec: &TemplatePoolSpec,
    ) -> CoreResult<Result<Vec<ResolvedTemplatePoolMember>, MutationError>> {
        let mut resolved = Vec::with_capacity(spec.members.len());
        for member in &spec.members {
            let pin = match self
                .resolve_template_ref(&member.template_profile_ref)
                .await
            {
                Ok(pin) => pin,
                Err(e) => return Ok(Err(unprocessable(e.code, e.summary))),
            };
            let policy = self
                .store
                .template_revision_get(&pin.0, pin.1)
                .await?
                .and_then(|r| r.fleet_input_policy_json)
                .unwrap_or_else(|| "{}".into());
            let schema = self.store.artifact_parameter_schema(&pin.2).await?;
            if schema.trim().is_empty() {
                return Err(CoreError::new(
                    ReasonCode::StorageUnavailable,
                    "pool member artifact has a blank parameter schema document",
                ));
            }
            if let Err(e) = super::validate_inputs(&member.template_inputs, &policy, Some(&schema))
            {
                return Ok(Err(unprocessable(e.code, e.summary)));
            }
            let inputs_digest = format!(
                "sha256:{}",
                hex::encode(sha2::Sha256::digest(
                    serde_json::to_vec(&member.template_inputs).unwrap_or_default()
                ))
            );
            resolved.push(ResolvedTemplatePoolMember {
                key: member.key.clone(),
                template_profile_key: pin.0,
                template_revision: pin.1,
                template_artifact_digest: pin.2,
                template_attestation_id: pin.3,
                template_inputs: member.template_inputs.clone(),
                inputs_digest,
                weight: member.weight,
                max_runners: member.max_runners,
            });
        }
        Ok(Ok(resolved))
    }
}

#[async_trait]
impl TemplatePoolRegistryPort for ControlPlane {
    #[tracing::instrument(name = "shaula.registry.template_pool_put", skip_all, fields(key = %key))]
    async fn template_pool_put(
        &self,
        actor: &Actor,
        key: &str,
        spec: TemplatePoolSpec,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::TemplatePublish) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.publish scope",
            )));
        }
        if !is_stable_identifier(key) || key.len() > 128 {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "template pool key must be a stable identifier of at most 128 characters",
            )));
        }
        if let Err(e) = spec.validate() {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }

        let canonical_body = serde_json::to_string(&spec)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        let precondition = match (&if_match, if_none_match) {
            (Some((incarnation, revision)), _) => format!("if-match:{incarnation}:{revision}"),
            (None, true) => "if-none-match:*".to_string(),
            (None, false) => "none".to_string(),
        };
        match self
            .idempotency_replay(
                "template_pool",
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

        let existing = self.store.template_pool_get(key).await?;
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
            }
            (Some(_), false, None) => return Ok(Err(MutationError::PreconditionRequired)),
        }

        // Admission always resolves current Active (spec 0037 §3): the
        // operator PUT and the follow cascade agree on the target, so
        // there is no retained-pin shortcut like the Fleet path.
        let members = match self.resolve_pool_members(&spec).await? {
            Ok(members) => members,
            Err(e) => return Ok(Err(e)),
        };

        // No-op shape: identical canonical spec AND identical resolved
        // members — a replayable 200 with the current ETag and NO new
        // revision. Any Active movement re-pins, which is a Replace.
        if let Some(current) = &existing {
            if let Some(latest) = self.store.template_pool_revision_latest(key).await? {
                let same_members = latest.members.len() == members.len()
                    && latest.members.iter().zip(members.iter()).all(|(a, b)| {
                        a.key == b.key
                            && a.template_profile_key == b.template_profile_key
                            && a.template_revision == b.template_revision
                            && a.template_artifact_digest == b.template_artifact_digest
                            && a.template_attestation_id == b.template_attestation_id
                            && a.inputs_digest == b.inputs_digest
                            && a.weight == b.weight
                            && a.max_runners == b.max_runners
                    });
                if latest.spec_json == canonical_body && same_members {
                    return Ok(Ok(MutationAccepted {
                        etag: format!("{}:{}", current.incarnation, current.desired_revision),
                        change: ChangeView {
                            id: String::new(),
                            resource_kind: "template_pool".into(),
                            resource_key: key.to_string(),
                            revision: current.desired_revision,
                            kind: "NoOp".into(),
                            state: "NoOp".into(),
                            reason: None,
                        },
                        no_op: true,
                    }));
                }
            }
        }

        let revision = existing
            .as_ref()
            .map(|p| p.desired_revision + 1)
            .unwrap_or(1);
        let incarnation = existing
            .as_ref()
            .map(|p| p.incarnation.clone())
            .unwrap_or_else(|| self.new_id());
        let kind = if existing.is_some() {
            "Replace"
        } else {
            "Create"
        };
        let now = self.now_ms();
        let change_id = self.new_id();
        let change = ChangeView {
            id: change_id.clone(),
            resource_kind: "template_pool".into(),
            resource_key: key.to_string(),
            revision,
            kind: kind.into(),
            state: "Pending".into(),
            reason: None,
        };
        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{revision}"),
            change: change.clone(),
            no_op: false,
        };
        let idempotency = idempotency_key.map(|idem| {
            let request_hash =
                self.idempotency_hash("template_pool", key, &idem, &canonical_body, &precondition);
            let response_body = serde_json::to_string(&accepted)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
            Ok((idem, request_hash, 202, response_body))
        });
        let idempotency = match idempotency {
            Some(Ok(value)) => Some(value),
            Some(Err(e)) => return Err(e),
            None => None,
        };
        // A pool owns no runners, so its commits race only other pool PUTs;
        // no effect gate or occupancy barrier applies (spec 0037 §5).
        let inputs_digest = format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(canonical_body.as_bytes()))
        );
        let facts = MutationFacts {
            resource_kind: "template_pool",
            resource_key: key.to_string(),
            incarnation: incarnation.clone(),
            revision,
            spec_json: canonical_body,
            template: None,
            template_pool: members,
            template_pool_ref: None,
            auth_desired: None,
            inputs_digest,
            actor: actor.name.clone(),
            now,
            change,
            outbox_topic: "template_pool.change".to_string(),
            outbox_payload: format!("{{\"change\":\"{change_id}\"}}"),
            idempotency,
        };
        if let Err(fence) = self.store.commit_template_pool_mutation(facts).await? {
            return Ok(Err(fence));
        }
        Ok(Ok(accepted))
    }

    async fn template_pool_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<TemplatePoolResource, MutationError>> {
        let Some(head) = self.store.template_pool_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if head.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let Some(latest) = self.store.template_pool_revision_latest(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let spec: TemplatePoolSpec = serde_json::from_str(&latest.spec_json)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        Ok(Ok(TemplatePoolResource {
            key: key.to_string(),
            spec,
            incarnation: head.incarnation,
            revision: head.desired_revision,
            resolved_members: latest.members,
            created_at: latest.created_at,
            updated_at: latest.created_at,
        }))
    }

    async fn template_pool_list(&self, _actor: &Actor) -> CoreResult<Vec<(String, i64, String)>> {
        self.store.template_pool_list().await
    }

    async fn template_pool_delete(
        &self,
        actor: &Actor,
        key: &str,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::TemplateRetire) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.retire scope",
            )));
        }
        let precondition = match &if_match {
            Some((incarnation, revision)) => format!("if-match:{incarnation}:{revision}"),
            None => "none".to_string(),
        };
        match self
            .idempotency_replay(
                "template_pool",
                key,
                &idempotency_key,
                "delete",
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
        let Some(head) = self.store.template_pool_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if head.incarnation != incarnation || head.desired_revision != revision {
            return Ok(Err(MutationError::PreconditionFailed {
                current: (head.incarnation.clone(), head.desired_revision),
            }));
        }
        if head.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let now = self.now_ms();
        let change_id = self.new_id();
        let new_revision = head.desired_revision + 1;
        let change = ChangeView {
            id: change_id.clone(),
            resource_kind: "template_pool".into(),
            resource_key: key.to_string(),
            revision: new_revision,
            kind: "Delete".into(),
            state: "Pending".into(),
            reason: None,
        };
        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{new_revision}"),
            change: change.clone(),
            no_op: false,
        };
        let idempotency = idempotency_key.map(|idem| {
            let request_hash =
                self.idempotency_hash("template_pool", key, &idem, "delete", &precondition);
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
            resource_kind: "template_pool",
            resource_key: key.to_string(),
            incarnation: head.incarnation.clone(),
            revision: new_revision,
            spec_json: "{}".to_string(),
            template: None,
            template_pool: Vec::new(),
            template_pool_ref: None,
            auth_desired: None,
            inputs_digest: "delete".to_string(),
            actor: actor.name.clone(),
            now,
            change,
            outbox_topic: "template_pool.change".to_string(),
            outbox_payload: format!("{{\"change\":\"{change_id}\"}}"),
            idempotency,
        };
        // The reference check (any live fleet's latest revision routing
        // through this pool blocks the tombstone) runs INSIDE the commit
        // transaction (spec 0037 §7).
        if let Err(blocked) = self.store.commit_template_pool_delete(facts).await? {
            return Ok(Err(blocked));
        }
        Ok(Ok(accepted))
    }

    async fn template_pool_change_get(
        &self,
        _actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
        self.store.template_pool_change_get(change_id).await
    }
}
