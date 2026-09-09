//! The Template Profile conditional PUT implementation (R9-02), split
//! to keep service_profile_registry.rs within the 400-line limit
//! (AGENTS.md).

use super::shaula_template_manifest;
use super::{request_hash, unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, ChangeView, MutationAccepted, MutationError, MutationFacts, Scope, TemplateProfilePut,
};

impl ControlPlane {
    pub(crate) async fn template_put_impl(
        &self,
        actor: &Actor,
        key: &str,
        payload: TemplateProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        // Authorization BEFORE any replay/conflict classification: a caller
        // without the publish scope must never learn stored state through
        // an idempotency key it observed (fleet/auth PUTs are scope-first).
        if !actor.has(Scope::TemplatePublish) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.publish scope",
            )));
        }
        // Request-boundary key validation (R7-05): only stable
        // identifiers become durable identity/idempotency material —
        // control bytes such as NUL are rejected here, not hashed.
        if let Err(e) = shaula_core::auth::validate_profile_key_for_fleet_ref(key) {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }
        // Idempotent replay: hash covers NON-secret members (artifact,
        // engine, policy); bindings are compared in protected memory
        // against the stored immutable Revision (spec 0005 section 3).
        let canonical = format!(
            "{}|{}|{}",
            payload.artifact_digest,
            payload.engine_ref,
            serde_json::to_string(&payload.fleet_input_policy).unwrap_or_default(),
        );
        if let Some(idem) = &idempotency_key {
            let hash = request_hash(&[
                b"template_profile",
                key.as_bytes(),
                idem.as_bytes(),
                canonical.as_bytes(),
            ]);
            match self
                .store
                .idempotency_find("template_profile", key, idem, &hash)
                .await?
            {
                shaula_core::registry::IdempotencyLookup::Miss => {}
                shaula_core::registry::IdempotencyLookup::Conflict => {
                    return Ok(Err(MutationError::IdempotencyConflict));
                }
                shaula_core::registry::IdempotencyLookup::Replay(body) => {
                    let accepted: MutationAccepted = serde_json::from_str(&body)
                        .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
                    let revision = accepted.change.revision;
                    let stored = self
                        .store
                        .template_protected_bindings(key, revision)
                        .await?
                        .map(|(json, _)| json)
                        .unwrap_or_default();
                    let submitted = serde_json::to_string(&payload.bindings).unwrap_or_default();
                    if stored != submitted {
                        return Ok(Err(MutationError::IdempotencyConflict));
                    }
                    return Ok(Ok(accepted));
                }
            }
        }
        let existing = self.store.template_profile_get(key).await?;
        if existing
            .as_ref()
            .is_some_and(|p| p.status == "Retiring" || p.status == "Retired")
        {
            return Ok(Err(MutationError::RetirementBlocked {
                reason: "profile is retiring".into(),
            }));
        }
        if let Err(error) = super::profile_conditions::check(
            existing
                .as_ref()
                .map(|p| (p.incarnation.as_str(), p.desired_revision)),
            if_none_match,
            if_match.as_ref(),
        ) {
            return Ok(Err(error));
        }
        // The artifact must be published, digest-addressed and shape-safe.
        let Some(manifest_yaml) = self
            .store
            .artifact_manifest(&payload.artifact_digest)
            .await?
        else {
            return Ok(Err(unprocessable(
                ReasonCode::TemplateInvalid,
                "artifact digest is not published",
            )));
        };
        let manifest = shaula_template_manifest(&manifest_yaml)?;
        let shape_ok = self
            .store
            .artifact_shape_ok(&payload.artifact_digest)
            .await?;
        if !shape_ok {
            return Ok(Err(unprocessable(
                ReasonCode::TemplateInvalid,
                "artifact shape rejected",
            )));
        }

        // Platform identity is fixed per incarnation; incompatible
        // manifest changes need a new profile key. The authority is the
        // INCARNATION FOUNDER (revision 1) via its immutable artifact
        // manifest — never the current desired revision: a pending
        // Candidate whose scan has not recorded platform/contract yet
        // would otherwise skip the comparison and let an incompatible
        // artifact slip into the same incarnation (F09, spec 0005 §5).
        if existing.is_some() {
            let founder = self
                .store
                .template_revision_get(key, 1)
                .await?
                .ok_or_else(|| {
                    CoreError::new(
                        ReasonCode::Internal,
                        "incarnation founder revision row missing",
                    )
                })?;
            let founder_manifest_text = self
                .store
                .artifact_manifest(&founder.artifact_digest)
                .await?
                .ok_or_else(|| {
                    CoreError::new(
                        ReasonCode::Internal,
                        "incarnation founder artifact is not published",
                    )
                })?;
            let founder_manifest = shaula_template_manifest(&founder_manifest_text)?;
            if founder_manifest.platform != manifest.platform
                || founder_manifest.bindings_contract != manifest.bindings_contract
            {
                return Ok(Err(MutationError::IdentityConflict));
            }
        }

        // No-op re-assertion (R9-02, spec 0005 §3): identical content
        // against the CURRENT desired revision is a durable 200 with NO
        // new Revision — a fresh idempotency key alone must not mint
        // revisions. Bindings are compared in protected memory.
        if let Some(head) = &existing {
            if let Some(current) = self
                .store
                .template_revision_get(key, head.desired_revision)
                .await?
            {
                let policy_json =
                    serde_json::to_string(&payload.fleet_input_policy).unwrap_or_default();
                let bindings_json = serde_json::to_string(&payload.bindings).unwrap_or_default();
                // Bindings plaintext lives behind the protected-memory
                // seam; the no-op comparison happens there too.
                let stored_bindings = self
                    .store
                    .template_protected_bindings(key, head.desired_revision)
                    .await?
                    .map(|(json, _)| json)
                    .unwrap_or_default();
                if current.artifact_digest == payload.artifact_digest
                    && current.engine_ref == payload.engine_ref
                    && current.fleet_input_policy_json.as_deref() == Some(policy_json.as_str())
                    && stored_bindings == bindings_json
                {
                    let accepted = MutationAccepted {
                        etag: format!("{}:{}", head.incarnation, head.desired_revision),
                        change: ChangeView {
                            id: String::new(),
                            resource_kind: "template_profile".to_string(),
                            resource_key: key.to_string(),
                            revision: head.desired_revision,
                            kind: "NoOp".to_string(),
                            state: "NoOp".to_string(),
                            reason: None,
                        },
                        no_op: true,
                    };
                    // A concurrent PUT that advanced the head since
                    // classification invalidates the no-op: surface the
                    // precondition, never record a stale 200.
                    let idempotency = idempotency_key
                        .as_ref()
                        .map(|idem| (idem.clone(), canonical.clone()));
                    match self
                        .store
                        .commit_template_noop(
                            key,
                            &head.incarnation,
                            head.desired_revision,
                            &actor.name,
                            idempotency,
                            self.now_ms(),
                        )
                        .await?
                    {
                        Ok(()) => return Ok(Ok(accepted)),
                        Err(mutation) => return Ok(Err(mutation)),
                    }
                }
            }
        }

        // Old immutable revisions remain readable and replayable, but a new
        // revision cannot opt out of the current official-container policy.
        if let Err(error) = manifest.validate_new_container_profile() {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        let now = self.now_ms();
        let incarnation = existing
            .as_ref()
            .map(|p| p.incarnation.clone())
            .unwrap_or_else(|| self.new_id());
        let revision = existing
            .as_ref()
            .map(|p| p.desired_revision + 1)
            .unwrap_or(1);
        let bindings_json = serde_json::to_string(&payload.bindings)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        let bindings_digest = shaula_core::template::BindingsDigest::from_keyed_material(
            &format!("{key}/{revision}"),
            &self.bindings_server_key,
        )?
        .0;
        let change_id = self.new_id();

        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{revision}"),
            change: ChangeView {
                id: change_id.clone(),
                resource_kind: "template_profile".to_string(),
                resource_key: key.to_string(),
                revision,
                kind: "Publish".to_string(),
                state: "Pending".to_string(),
                reason: None,
            },
            no_op: false,
        };
        // Template PUT idempotency: hash covers non-secret members;
        // bindings are compared in protected memory against the stored
        // immutable Revision on replay (spec 0005 §3).
        let idempotency = idempotency_key.map(|idem| {
            let request_hash = request_hash(&[
                b"template_profile",
                key.as_bytes(),
                idem.as_bytes(),
                canonical.as_bytes(),
            ]);
            let response_body = serde_json::to_string(&accepted)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
            Ok((idem, request_hash, 202, response_body))
        });
        let idempotency = match idempotency {
            Some(Ok(v)) => Some(v),
            Some(Err(e)) => return Err(e),
            None => None,
        };

        let facts = MutationFacts {
            resource_kind: "template_profile",
            resource_key: key.to_string(),
            incarnation: incarnation.clone(),
            revision,
            spec_json: manifest_yaml.clone(),
            template: None,
            auth_desired: None,
            inputs_digest: payload.artifact_digest.clone(),
            actor: actor.name.clone(),
            now,
            change: accepted.change.clone(),
            outbox_topic: "profile.validate".to_string(),
            outbox_payload: format!("{{\"key\":\"{key}\",\"revision\":{revision}}}"),
            idempotency,
        };
        let extra = (
            payload.engine_ref,
            bindings_json,
            bindings_digest,
            serde_json::to_string(&payload.fleet_input_policy)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?,
        );
        // A lost fence race surfaces as a precondition failure so the
        // client re-reads the current desired head; never overwrite
        // (R9-02).
        match self.store.commit_template_revision(facts, extra).await? {
            Ok(()) => Ok(Ok(accepted)),
            Err(mutation) => Ok(Err(mutation)),
        }
    }
}
