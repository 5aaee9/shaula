//! Explicit published-template updates reuse immutable configuration server-side.

use super::{request_hash, unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::bindings_projection::BindingsSchema;
use shaula_core::registry::{
    Actor, MutationAccepted, MutationError, Scope, TemplateProfilePut, TemplateProfileUpdate,
};
use tracing::warn;

/// Admission shares normal conditional publication and durable replay.
pub(super) struct TemplatePublication {
    pub payload: TemplateProfilePut,
    pub if_none_match: bool,
    pub if_match: Option<(String, i64)>,
    pub idempotency_key: Option<String>,
    pub update_identity: Option<String>,
    pub update_base_source: Option<String>,
}

impl TemplatePublication {
    pub(super) fn canonical(&self) -> String {
        self.update_identity.clone().unwrap_or_else(|| {
            match &self.payload.source_key {
                Some(source_key) => serde_json::json!({
                    "operation": "template_profile_put",
                    "source_key": source_key,
                    "artifact_digest": self.payload.artifact_digest,
                    "engine_ref": self.payload.engine_ref,
                    "fleet_input_policy": self.payload.fleet_input_policy,
                })
                .to_string(),
                // Existing absent-source requests keep their durable identity.
                None => format!(
                    "{}|{}|{}",
                    self.payload.artifact_digest,
                    self.payload.engine_ref,
                    serde_json::to_string(&self.payload.fleet_input_policy).unwrap_or_default(),
                ),
            }
        })
    }
}

impl ControlPlane {
    /// The artifact's bindings schema as a sensitivity authority. Any
    /// acquisition or parse problem fails closed to an all-sensitive
    /// view (spec 0038 §2): reads degrade to presence markers, never
    /// leak, and never 500 on a schema problem.
    pub(super) async fn bindings_schema_view(&self, digest: &str) -> BindingsSchema {
        match self.store.artifact_bindings_schema(digest).await {
            Ok(Some(text)) => match serde_json::from_str(&text) {
                Ok(document) => BindingsSchema::parse(&document).unwrap_or_else(|error| {
                    warn!(digest, %error, "bindings schema unparsable; failing closed");
                    BindingsSchema::all_sensitive()
                }),
                Err(error) => {
                    warn!(digest, %error, "bindings schema unreadable; failing closed");
                    BindingsSchema::all_sensitive()
                }
            },
            Ok(None) => BindingsSchema::all_sensitive(),
            Err(error) => {
                warn!(digest, %error, "bindings schema unavailable; failing closed");
                BindingsSchema::all_sensitive()
            }
        }
    }

    /// Resolves an Update's optional `bindings` against the base
    /// revision and the TARGET artifact's schema (spec 0038 §3):
    /// per-field keep/replace merge followed by structural validation,
    /// rejecting any submission that would write an unknown field, a
    /// presence-marker echo or an invalid merged set. Returns the merged
    /// bindings plus the non-secret identity view of what was supplied
    /// (sensitive values appear only as replaced/kept booleans).
    async fn resolve_update_bindings(
        &self,
        base_json: &str,
        submitted: &serde_json::Map<String, serde_json::Value>,
        target_digest: &str,
    ) -> CoreResult<Result<(serde_json::Value, serde_json::Value), MutationError>> {
        let base: serde_json::Map<String, serde_json::Value> = serde_json::from_str(base_json)
            .map_err(|_| stored_configuration_error("stored template bindings are invalid"))?;
        let schema_text = self.store.artifact_bindings_schema(target_digest).await?;
        let schema = match schema_text
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
        {
            Some(Ok(document)) => match BindingsSchema::parse(&document) {
                Ok(schema) => schema,
                Err(_) => {
                    return Ok(Err(unprocessable(
                        ReasonCode::TemplateInvalid,
                        "target artifact bindings schema is invalid",
                    )))
                }
            },
            _ => {
                // Unreadable/absent schema: only an all-keep submission may
                // proceed; any supplied value would bypass validation.
                if submitted.values().all(serde_json::Value::is_null) {
                    let identity = serde_json::json!({"supplied": true, "secrets": "kept"});
                    return Ok(Ok((serde_json::Value::Object(base), identity)));
                }
                return Ok(Err(unprocessable(
                    ReasonCode::TemplateInvalid,
                    "target artifact bindings schema is unavailable",
                )));
            }
        };
        let merged = match schema.merge_update(&base, submitted) {
            Ok(merged) => merged,
            Err(error) => return Ok(Err(unprocessable(error.code, error.summary))),
        };
        if let Err(error) = schema.validate(&merged) {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        // Identity is the non-secret view of the MERGED set: omission and
        // the null sentinel normalize to the same identity (both keep),
        // and a replaced secret appears only as a boolean — secret value
        // differences are caught in protected memory on replay.
        let mut identity = serde_json::Map::new();
        for (key, value) in &merged {
            let entry = if schema.sensitive(key) {
                serde_json::json!({
                    "replaced": submitted.get(key).is_some_and(|v| !v.is_null())
                })
            } else {
                value.clone()
            };
            identity.insert(key.clone(), entry);
        }
        Ok(Ok((
            serde_json::Value::Object(merged),
            serde_json::Value::Object(identity),
        )))
    }

    pub(super) async fn template_put_impl(
        &self,
        actor: &Actor,
        key: &str,
        publication: TemplatePublication,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        let canonical = publication.canonical();
        let idem = publication.idempotency_key.clone();
        let submitted = serde_json::to_string(&publication.payload.bindings)
            .map_err(|_| stored_configuration_error("template bindings could not be encoded"))?;
        let result = self.template_publish_admit(actor, key, publication).await;
        // Two requests can both observe an idempotency miss before one wins
        // the transaction. Reclassify against its durable result, including
        // NoOp's unique-key race. This only reads; it never retries a mutation.
        if actor.has(Scope::TemplatePublish)
            && matches!(
                &result,
                Ok(Err(MutationError::PreconditionFailed { .. })) | Err(_)
            )
        {
            if let Some(idem) = idem {
                match self
                    .template_publication_replay(key, &idem, &canonical, &submitted)
                    .await?
                {
                    Ok(Some(accepted)) => return Ok(Ok(accepted)),
                    Err(mutation) => return Ok(Err(mutation)),
                    Ok(None) => {}
                }
            }
        }
        result
    }

    pub(super) async fn template_publication_replay(
        &self,
        key: &str,
        idem: &str,
        canonical: &str,
        submitted_bindings: &str,
    ) -> CoreResult<Result<Option<MutationAccepted>, MutationError>> {
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
            shaula_core::registry::IdempotencyLookup::Miss => Ok(Ok(None)),
            shaula_core::registry::IdempotencyLookup::Conflict => {
                Ok(Err(MutationError::IdempotencyConflict))
            }
            shaula_core::registry::IdempotencyLookup::Replay(body) => {
                let accepted: MutationAccepted = serde_json::from_str(&body).map_err(|_| {
                    stored_configuration_error("stored publication result is invalid")
                })?;
                let stored = self
                    .store
                    .template_protected_bindings(key, accepted.change.revision)
                    .await?
                    .map(|(json, _)| json)
                    .unwrap_or_default();
                if stored != submitted_bindings {
                    return Ok(Err(MutationError::IdempotencyConflict));
                }
                Ok(Ok(Some(accepted)))
            }
        }
    }

    pub(super) async fn template_update_impl(
        &self,
        actor: &Actor,
        key: &str,
        payload: TemplateProfileUpdate,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::TemplatePublish) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.publish scope",
            )));
        }
        if let Err(error) = shaula_core::auth::validate_profile_key_for_fleet_ref(key) {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        let Some(expected) = if_match else {
            return Ok(Err(MutationError::PreconditionRequired));
        };
        let Some(head) = self.store.template_profile_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if head.incarnation != expected.0 {
            return Ok(Err(MutationError::PreconditionFailed {
                current: (head.incarnation, head.desired_revision),
            }));
        }
        let Some(base) = self.store.template_revision_get(key, expected.1).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        // The request identity retains whether policy and bindings were
        // supplied, as well as the base. Ordinary PUTs cannot replay an
        // Update's idempotency key. The bindings member carries only the
        // non-secret view of the submission; secret equality stays in
        // protected memory on replay (spec 0038 §3).
        let mut update_identity = serde_json::json!({
            "operation": "template_profile_update",
            "base": expected,
            "artifact_digest": payload.artifact_digest,
            "engine_ref": payload.engine_ref,
            "fleet_input_policy": payload.fleet_input_policy,
        });
        // Omit the member entirely for compatibility with pre-source replays.
        if let Some(source_key) = &payload.source_key {
            update_identity["source_key"] = source_key.clone().into();
        }
        let Some((bindings_json, _)) = self
            .store
            .template_protected_bindings(key, expected.1)
            .await?
        else {
            return Err(stored_configuration_error(
                "template bindings are unavailable",
            ));
        };
        let bindings = match payload.bindings.as_ref() {
            Some(submitted) => {
                match self
                    .resolve_update_bindings(&bindings_json, submitted, &payload.artifact_digest)
                    .await?
                {
                    Ok((merged, identity)) => {
                        update_identity["bindings"] = identity;
                        merged
                    }
                    Err(mutation) => return Ok(Err(mutation)),
                }
            }
            None => serde_json::from_str(&bindings_json)
                .map_err(|_| stored_configuration_error("stored template bindings are invalid"))?,
        };
        let fleet_input_policy = match payload.fleet_input_policy {
            Some(policy) => serde_json::Value::Object(policy),
            None => serde_json::from_str(base.fleet_input_policy_json.as_deref().unwrap_or("null"))
                .map_err(|_| stored_configuration_error("stored template policy is invalid"))?,
        };
        self.template_put_impl(
            actor,
            key,
            TemplatePublication {
                payload: TemplateProfilePut {
                    source_key: payload.source_key,
                    artifact_digest: payload.artifact_digest,
                    engine_ref: payload.engine_ref,
                    bindings,
                    fleet_input_policy,
                },
                if_none_match: false,
                if_match: Some(expected),
                idempotency_key,
                update_identity: Some(update_identity.to_string()),
                update_base_source: base.source_key,
            },
        )
        .await
    }
}

fn stored_configuration_error(summary: &str) -> CoreError {
    CoreError::new(ReasonCode::Internal, summary)
}
