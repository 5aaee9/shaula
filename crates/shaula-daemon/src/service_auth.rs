//! Auth Profile mutation/reads split out of the trait impl to keep file
//! sizes within the 400-line limit (AGENTS.md).

use super::{request_hash, unprocessable, ControlPlane};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, AuthProfilePut, AuthProfileView, AuthRevisionRow, ChangeView, MutationAccepted,
    MutationError, MutationFacts, Scope,
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
        // Idempotent replay for profile mutations (0005 sec 3). The hash
        // includes a canonical body fingerprint over every NON-secret
        // semantic member — identity fields AND the target allowlist — so
        // changed content of any of them conflicts instead of silently
        // replaying (spec 0005 section 3/§2).
        let allowlist = match shaula_core::auth::TargetAllowlist::new(payload.allowlist.clone()) {
            Ok(allowlist) => allowlist,
            Err(e) => return Ok(Err(unprocessable(e.code, e.summary))),
        };
        let allowlist_json = serde_json::to_string(&allowlist)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        let canonical_body = format!(
            "{}|{}|{}|{}|{}",
            payload.kind.as_str(),
            payload.app_id.as_deref().unwrap_or(""),
            payload.installation_id.unwrap_or(0),
            payload.pat_identity.as_deref().unwrap_or(""),
            allowlist_json,
        );
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
        // Identity fields are immutable per incarnation: same-key PUT is
        // rotation only. Mismatched principal/identity needs a new key.
        if let Some(current) = &existing {
            let Some(active) = self
                .store
                .auth_revision_get(key, current.desired_revision)
                .await?
            else {
                return Ok(Err(MutationError::IdentityConflict));
            };
            if active.kind != payload.kind.as_str() {
                return Ok(Err(MutationError::IdentityConflict));
            }
            let identity_matches = match payload.kind {
                shaula_core::auth::AuthKind::GithubApp => {
                    active.app_id == payload.app_id
                        && active.installation_id == payload.installation_id
                }
                shaula_core::auth::AuthKind::Pat => active.pat_principal == payload.pat_identity,
            };
            if !identity_matches || active.allowlist_json != allowlist_json {
                return Ok(Err(MutationError::IdentityConflict));
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

        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{revision}"),
            change: ChangeView {
                id: change_id.clone(),
                resource_kind: "github_auth_profile".to_string(),
                resource_key: key.to_string(),
                revision,
                kind: "Rotate".to_string(),
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
            kind: payload.kind.as_str().to_string(),
            app_id: payload.app_id.clone(),
            installation_id: payload.installation_id,
            pat_principal: payload.pat_identity.clone(),
            allowlist_json: allowlist_json.clone(),
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

    pub(crate) async fn auth_get_impl(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<AuthProfileView, MutationError>> {
        let Some(profile) = self.store.auth_profile_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let active = self.store.auth_revision_active(key).await?;
        Ok(Ok(AuthProfileView {
            key: key.to_string(),
            incarnation: profile.incarnation.clone(),
            desired_revision: profile.desired_revision,
            active_revision: profile.active_revision,
            status: profile.status.clone(),
            kind: active.as_ref().map(|r| {
                if r.kind == "pat" {
                    shaula_core::auth::AuthKind::Pat
                } else {
                    shaula_core::auth::AuthKind::GithubApp
                }
            }),
            identity: active.as_ref().map(|r| {
                if let Some(principal) = &r.pat_principal {
                    principal.clone()
                } else {
                    format!(
                        "app/{}/installation/{}",
                        r.app_id.clone().unwrap_or_default(),
                        r.installation_id.unwrap_or_default()
                    )
                }
            }),
            credential_present: active.is_some(),
            target_allowlist: active
                .as_ref()
                .map(|r| {
                    serde_json::from_str::<shaula_core::auth::TargetAllowlist>(&r.allowlist_json)
                        .map(|a| a.targets.iter().map(|t| t.config_url()).collect())
                        .unwrap_or_default()
                })
                .unwrap_or_default(),
        }))
    }
}

impl ControlPlane {
    /// Applies the outcome of GitHub identity/access validation for an
    /// Auth Candidate (staged activation: only then does the active head
    /// advance). Called by the async credential validator (phase 3).
    pub async fn auth_apply_validation(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .auth_apply_validation(key, revision, accepted, reason, now)
            .await
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
