//! Explicit published-template updates reuse immutable configuration server-side.

use super::{request_hash, unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, MutationAccepted, MutationError, Scope, TemplateProfilePut, TemplateProfileUpdate,
};

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
        // The request identity retains whether policy was supplied, as well as
        // the base. Ordinary PUTs cannot replay an Update's idempotency key.
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
        let bindings = serde_json::from_str(&bindings_json)
            .map_err(|_| stored_configuration_error("stored template bindings are invalid"))?;
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
