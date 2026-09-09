//! Policy-only publication: the store owns exact Active credential inheritance.

use shaula_core::auth_policy::TargetPolicy;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, AuthPolicyUpdate, ChangeView, MutationAccepted, MutationError, MutationFacts, Scope,
};

use super::{request_hash, unprocessable, ControlPlane};

impl ControlPlane {
    pub(crate) async fn auth_policy_update_impl(
        &self,
        actor: &Actor,
        key: &str,
        payload: AuthPolicyUpdate,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::AuthRead) || !actor.has(Scope::AuthWrite) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "auth.read and auth.write scopes are required",
            )));
        }
        if let Err(error) = shaula_core::auth::validate_profile_key_for_fleet_ref(key) {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        if payload.base_revision <= 0 {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "base_revision must be a positive Active revision",
            )));
        }
        let policy = match TargetPolicy::new(payload.target_policy) {
            Ok(policy) => policy,
            Err(error) => return Ok(Err(unprocessable(error.code, error.summary))),
        };
        let Some((incarnation, expected_revision)) = if_match else {
            return Ok(Err(MutationError::PreconditionRequired));
        };
        let Some(revision) = expected_revision.checked_add(1).filter(|value| *value > 1) else {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "If-Match revision is outside the supported range",
            )));
        };
        let policy_json = serde_json::to_string(&policy)
            .map_err(|_| CoreError::new(ReasonCode::Internal, "policy encoding failed"))?;
        let canonical = serde_json::json!({
            "operation": "auth_policy_update",
            "base_revision": payload.base_revision,
            "target_policy": policy,
            "if_match": [incarnation, expected_revision],
        })
        .to_string();
        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{revision}"),
            change: ChangeView {
                id: self.new_id(),
                resource_kind: "github_auth_profile".into(),
                resource_key: key.into(),
                revision,
                kind: "Publish".into(),
                state: "Pending".into(),
                reason: None,
            },
            no_op: false,
        };
        let idempotency = idempotency_key
            .map(|idem| {
                let hash = request_hash(&[
                    b"github_auth_profile",
                    key.as_bytes(),
                    idem.as_bytes(),
                    canonical.as_bytes(),
                ]);
                let response = serde_json::to_string(&accepted).map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "publication result encoding failed")
                })?;
                Ok((idem, hash, 202, response))
            })
            .transpose()?;
        // No mutable head or credential is read here. The writer transaction
        // resolves an accepted replay before reading the current Active base.
        self.store
            .commit_auth_policy_update(
                MutationFacts {
                    resource_kind: "github_auth_profile",
                    resource_key: key.into(),
                    incarnation,
                    revision,
                    spec_json: String::new(),
                    template: None,
                    auth_desired: None,
                    inputs_digest: String::new(),
                    actor: actor.name.clone(),
                    now: self.now_ms(),
                    change: accepted.change,
                    outbox_topic: "profile.auth_validate".into(),
                    outbox_payload: serde_json::json!({"key": key, "revision": revision})
                        .to_string(),
                    idempotency,
                },
                payload.base_revision,
                policy_json,
            )
            .await
    }
}
