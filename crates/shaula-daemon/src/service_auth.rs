//! Auth Profile mutation/reads split out of the trait impl to keep file
//! sizes within the 400-line limit (AGENTS.md).

use super::{request_hash, unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, AuthProfilePut, AuthRevisionRow, ChangeView, MutationAccepted, MutationError,
    MutationFacts, Scope,
};

impl ControlPlane {
    pub(crate) async fn auth_put_impl(
        &self,
        actor: &Actor,
        key: &str,
        payload: AuthProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::AuthWrite) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing auth.write scope",
            )));
        }
        // Request-boundary key validation (R7-05): stable identifiers
        // only, so control bytes never reach identity/idempotency hashes.
        if let Err(e) = shaula_core::auth::validate_profile_key_for_fleet_ref(key) {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }
        // Format detection FIRST (spec 0011 §6): schema version is an
        // explicit choice; the parse validates every per-format member.
        let format = match crate::service_auth_format::AuthPutFormat::parse(&payload) {
            Ok(format) => format,
            Err((code, summary)) => return Ok(Err(unprocessable(code, summary))),
        };
        // Idempotent replay for profile mutations (0005 sec 3). The hash
        // includes a canonical body fingerprint over every NON-secret
        // semantic member of the parsed format — identity fields AND the
        // target policy — so changed content conflicts instead of
        // silently replaying (spec 0005 section 3/§2).
        let canonical_body = format.canonical_body();
        if let Some(idem) = &idempotency_key {
            let hash = request_hash(&[
                b"github_auth_profile",
                key.as_bytes(),
                idem.as_bytes(),
                canonical_body.as_bytes(),
            ]);
            match self
                .store
                .idempotency_find("github_auth_profile", key, idem, &hash)
                .await?
            {
                shaula_core::registry::IdempotencyLookup::Replay(body) => {
                    // Protected-memory secret comparison against the stored
                    // immutable Revision: a different secret means the caller
                    // changed content — conflict, never a silent replay
                    // (spec 0005 section 3). No secret-derived verifier is
                    // persisted.
                    let accepted: MutationAccepted = serde_json::from_str(&body)
                        .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
                    let revision = accepted.change.revision;
                    let stored = self
                        .store
                        .auth_credential_bytes(key, revision)
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
        // Only a supported active revision establishes this profile's App
        // identity. Historical formats cannot be upgraded or reused implicitly.
        let mut head_policy_json: Option<String> = None;
        if let Some(current) = &existing {
            let Some(head) = self
                .store
                .auth_revision_get(key, current.desired_revision)
                .await?
            else {
                return Ok(Err(MutationError::IdentityConflict));
            };
            if head.schema_version != 2 || head.kind != "github_app" {
                return Ok(Err(unprocessable(
                    ReasonCode::SpecInvalid,
                    "unsupported authentication profile; publish under a new profile key",
                )));
            }
            head_policy_json = head.policy_json;
            if let Some(active_revision) = current.active_revision {
                let Some(active) = self.store.auth_revision_get(key, active_revision).await? else {
                    return Ok(Err(MutationError::IdentityConflict));
                };
                if active.schema_version != 2 || active.kind != "github_app" {
                    return Ok(Err(unprocessable(
                        ReasonCode::SpecInvalid,
                        "unsupported authentication profile; publish under a new profile key",
                    )));
                }
                if active.app_id.as_deref() != Some(format.app_id.as_str()) {
                    return Ok(Err(MutationError::IdentityConflict));
                }
            }
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
        let change_id = self.new_id();
        // Policy publication and pure credential rotation are named
        // differently; both run the same CAS/validation flow (spec 0011 §6).
        let change_kind = if head_policy_json.as_deref() == Some(format.policy_json.as_str()) {
            "Rotate"
        } else {
            "Publish"
        };
        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{revision}"),
            change: ChangeView {
                id: change_id.clone(),
                resource_kind: "github_auth_profile".to_string(),
                resource_key: key.to_string(),
                revision,
                kind: change_kind.to_string(),
                state: "Pending".to_string(),
                reason: None,
            },
            no_op: false,
        };
        // Profile mutations honour the same idempotency contract (0005 section 3).
        let idempotency = idempotency_key.map(|idem| {
            let request_hash = request_hash(&[
                b"github_auth_profile",
                key.as_bytes(),
                idem.as_bytes(),
                canonical_body.as_bytes(),
            ]);
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
            resource_kind: "github_auth_profile",
            resource_key: key.to_string(),
            incarnation: incarnation.clone(),
            revision,
            spec_json: String::new(),
            template: None,
            auth_desired: None,
            inputs_digest: String::new(),
            actor: actor.name.clone(),
            now,
            change: accepted.change.clone(),
            outbox_topic: "profile.auth_validate".to_string(),
            outbox_payload: format!("{{\"key\":\"{key}\",\"revision\":{revision}}}"),
            idempotency,
        };
        let credential_row = AuthRevisionRow {
            profile_key: key.to_string(),
            revision,
            state: "Validating".into(),
            reason: None,
            kind: "github_app".into(),
            app_id: Some(format.app_id),
            schema_version: 2,
            policy_json: Some(format.policy_json),
            validation_snapshot_json: None,
        };
        if let Err(error) = self
            .store
            .commit_auth_revision(facts, credential_row, payload.secret.expose().as_bytes())
            .await?
        {
            return Ok(Err(error));
        }
        Ok(Ok(accepted))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
