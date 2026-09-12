//! Forgejo token publication. This deliberately uses the existing protected
//! credential column only as storage plumbing; its kind, target and
//! validation path are independent from GitHub App policy revisions.

use super::{request_hash, unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, AuthProfilePut, AuthRevisionRow, ChangeView, MutationAccepted, MutationError,
    MutationFacts,
};

impl ControlPlane {
    pub(super) async fn auth_put_forgejo(
        &self,
        actor: &Actor,
        key: &str,
        payload: AuthProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        let format = match crate::service_auth_format::ForgejoAuthPutFormat::parse(&payload) {
            Ok(format) => format,
            Err((code, summary)) => return Ok(Err(unprocessable(code, summary))),
        };
        let canonical_body = format.canonical_body();
        if let Some(idem) = &idempotency_key {
            let hash = request_hash(&[
                b"forgejo_auth_profile",
                key.as_bytes(),
                idem.as_bytes(),
                canonical_body.as_bytes(),
            ]);
            match self
                .store
                .idempotency_find("forgejo_auth_profile", key, idem, &hash)
                .await?
            {
                shaula_core::registry::IdempotencyLookup::Replay(body) => {
                    let accepted: MutationAccepted = serde_json::from_str(&body)
                        .map_err(|error| CoreError::new(ReasonCode::Internal, error.to_string()))?;
                    let stored = self
                        .store
                        .auth_credential_bytes(key, accepted.change.revision)
                        .await?
                        .unwrap_or_default();
                    if !constant_time_eq(&stored, payload.secret.expose().as_bytes()) {
                        return Ok(Err(MutationError::IdempotencyConflict));
                    }
                    return Ok(Ok(accepted));
                }
                shaula_core::registry::IdempotencyLookup::Conflict => {
                    return Ok(Err(MutationError::IdempotencyConflict));
                }
                shaula_core::registry::IdempotencyLookup::Miss => {}
            }
        }

        let existing = self.store.auth_profile_get(key).await?;
        if existing
            .as_ref()
            .is_some_and(|profile| profile.status == "Retiring" || profile.status == "Retired")
        {
            return Ok(Err(MutationError::RetirementBlocked {
                reason: "profile is retiring".into(),
            }));
        }
        if let Err(error) = super::profile_conditions::check(
            existing
                .as_ref()
                .map(|profile| (profile.incarnation.as_str(), profile.desired_revision)),
            if_none_match,
            if_match.as_ref(),
        ) {
            return Ok(Err(error));
        }

        if let Some(profile) = &existing {
            let Some(head) = self
                .store
                .auth_revision_get(key, profile.desired_revision)
                .await?
            else {
                return Ok(Err(MutationError::IdentityConflict));
            };
            if head.kind != "forgejo_token" || head.schema_version != 1 {
                return Ok(Err(MutationError::IdentityConflict));
            }
            let stored_target = head
                .policy_json
                .as_deref()
                .ok_or_else(|| CoreError::new(ReasonCode::Internal, "Forgejo target is missing"))
                .and_then(|json| {
                    serde_json::from_str::<shaula_core::forgejo::ForgejoTarget>(json).map_err(
                        |error| {
                            CoreError::new(
                                ReasonCode::Internal,
                                format!("Forgejo target is invalid: {error}"),
                            )
                        },
                    )
                })?;
            if stored_target != format.target {
                return Ok(Err(MutationError::IdentityConflict));
            }
        }

        let now = self.now_ms();
        let incarnation = existing
            .as_ref()
            .map(|profile| profile.incarnation.clone())
            .unwrap_or_else(|| self.new_id());
        let revision = existing
            .as_ref()
            .map(|profile| profile.desired_revision + 1)
            .unwrap_or(1);
        let change_id = self.new_id();
        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{revision}"),
            change: ChangeView {
                id: change_id.clone(),
                resource_kind: "forgejo_auth_profile".into(),
                resource_key: key.into(),
                revision,
                kind: if existing.is_some() {
                    "Rotate"
                } else {
                    "Publish"
                }
                .into(),
                state: "Pending".into(),
                reason: None,
            },
            no_op: false,
        };
        let idempotency = idempotency_key
            .map(|idem| {
                let request_hash = request_hash(&[
                    b"forgejo_auth_profile",
                    key.as_bytes(),
                    idem.as_bytes(),
                    canonical_body.as_bytes(),
                ]);
                let response_body = serde_json::to_string(&accepted)
                    .map_err(|error| CoreError::new(ReasonCode::Internal, error.to_string()))?;
                Ok((idem, request_hash, 202, response_body))
            })
            .transpose()?;
        let facts = MutationFacts {
            resource_kind: "forgejo_auth_profile",
            resource_key: key.into(),
            incarnation,
            revision,
            spec_json: String::new(),
            template: None,
            auth_desired: None,
            inputs_digest: String::new(),
            actor: actor.name.clone(),
            now,
            change: accepted.change.clone(),
            outbox_topic: "profile.auth_validate".into(),
            outbox_payload: format!(r#"{{"key":"{key}","revision":{revision}}}"#),
            idempotency,
        };
        let row = AuthRevisionRow {
            profile_key: key.into(),
            revision,
            state: "Validating".into(),
            reason: None,
            kind: "forgejo_token".into(),
            app_id: None,
            schema_version: 1,
            policy_json: Some(format.target_json),
            validation_snapshot_json: None,
        };
        match self
            .store
            .commit_auth_revision(facts, row, payload.secret.expose().as_bytes())
            .await?
        {
            Ok(()) => Ok(Ok(accepted)),
            Err(error) => Ok(Err(error)),
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}
