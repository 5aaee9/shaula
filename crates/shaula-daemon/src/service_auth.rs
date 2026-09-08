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
        let canonical_body = match format.canonical_body(&payload) {
            Ok(body) => body,
            Err(e) => return Err(e),
        };
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
        // Identity rules per format (spec 0011 §5.1/§7): the ESTABLISHED
        // identity is the ACTIVE revision — a rejected or in-flight
        // desired Candidate is history, never authorization (spec 0011
        // §7.4: activation, not publication, fixes the App identity). A
        // v2 App profile that has never activated establishes its identity
        // at first activation. Legacy publications retain their original
        // immutable principal/installation and exact scope. The downgrade guard still
        // follows the DESIRED head: the direction of travel can never
        // reverse, even while identity corrections are admitted.
        let mut head_policy_json: Option<String> = None;
        if let Some(current) = &existing {
            let Some(head) = self
                .store
                .auth_revision_get(key, current.desired_revision)
                .await?
            else {
                return Ok(Err(MutationError::IdentityConflict));
            };
            head_policy_json = head.policy_json.clone();
            let established = match current.active_revision {
                Some(active_revision) => {
                    match self.store.auth_revision_get(key, active_revision).await? {
                        Some(row) => Some(row),
                        None => return Ok(Err(MutationError::IdentityConflict)),
                    }
                }
                // Nothing has ever activated (G9): no identity authority
                // exists yet.
                None => None,
            };
            let identity_matches = match &format {
                crate::service_auth_format::AuthPutFormat::Legacy { allowlist_json } => {
                    if head.schema_version >= 2 {
                        // v2 active/desired heads are never downgraded to
                        // the legacy single-installation shape.
                        false
                    } else {
                        let established = established.as_ref().unwrap_or(&head);
                        established.kind == payload.kind.as_str()
                            && match payload.kind {
                                shaula_core::auth::AuthKind::GithubApp => {
                                    established.app_id == payload.app_id
                                        && established.installation_id == payload.installation_id
                                }
                                shaula_core::auth::AuthKind::Pat => {
                                    established.pat_principal == payload.pat_identity
                                }
                            }
                            && established.allowlist_json == *allowlist_json
                    }
                }
                crate::service_auth_format::AuthPutFormat::V2 { app_id, .. } => {
                    // Same App identity is REQUIRED against the ESTABLISHED
                    // (active) revision, including the legacy→v2 upgrade
                    // path (spec 0011 §7.4); the kind cannot change; the
                    // policy may change freely. A legacy revision that
                    // stores a client-ID STRING (e.g. `Iv23…`) cannot be
                    // compared literally against the required numeric v2
                    // App id: the explicit upgrade is admitted and the
                    // SAME-App continuity is proven asynchronously via
                    // `/app` client_id equality before promotion
                    // (spec 0011 §7.5). A different numeric App is always
                    // a conflict.
                    head.kind == shaula_core::auth::AuthKind::GithubApp.as_str()
                        && match established.as_ref().and_then(|e| e.app_id.as_deref()) {
                            Some(head_app) if head_app == app_id.as_str() => true,
                            Some(head_app)
                                if !head_app.is_empty()
                                    && head_app.bytes().any(|b| !b.is_ascii_digit()) =>
                            {
                                // Legacy client-ID form: upgrade admitted,
                                // continuity proven by the validator.
                                true
                            }
                            // Never activated: this publication establishes
                            // the App identity (G9).
                            _ => established.is_none(),
                        }
                }
            };
            if !identity_matches {
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
        // Policy publication and pure credential rotation are named
        // differently; both run the same CAS/validation flow (spec 0011 §6).
        let change_kind = match &format {
            crate::service_auth_format::AuthPutFormat::Legacy { .. } => "Rotate",
            crate::service_auth_format::AuthPutFormat::V2 { policy_json, .. } => {
                if head_policy_json.as_deref() == Some(policy_json.as_str()) {
                    "Rotate"
                } else {
                    "Publish"
                }
            }
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
        let credential_row = match &format {
            crate::service_auth_format::AuthPutFormat::Legacy { allowlist_json } => {
                AuthRevisionRow {
                    profile_key: key.to_string(),
                    revision,
                    state: "Validating".into(),
                    reason: None,
                    kind: payload.kind.as_str().to_string(),
                    app_id: payload.app_id.clone(),
                    installation_id: payload.installation_id,
                    pat_principal: payload.pat_identity.clone(),
                    allowlist_json: allowlist_json.clone(),
                    schema_version: 1,
                    policy_json: None,
                    validation_snapshot_json: None,
                }
            }
            crate::service_auth_format::AuthPutFormat::V2 {
                app_id,
                policy_json,
            } => {
                // A v2 row carries the policy, never the legacy allowlist:
                // an empty allowlist makes an old binary fail closed
                // instead of misreading the row (spec 0011 §7.7).
                AuthRevisionRow {
                    profile_key: key.to_string(),
                    revision,
                    state: "Validating".into(),
                    reason: None,
                    kind: shaula_core::auth::AuthKind::GithubApp.as_str().to_string(),
                    app_id: Some(app_id.clone()),
                    installation_id: None,
                    pat_principal: None,
                    allowlist_json: String::new(),
                    schema_version: 2,
                    policy_json: Some(policy_json.clone()),
                    validation_snapshot_json: None,
                }
            }
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
