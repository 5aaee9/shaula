//! Auth PUT format detection and validation (spec 0011 §3/§6). The schema
//! version is an EXPLICIT choice: absent version means legacy, and no
//! payload shape (wildcard or not) is ever allowed to guess one.

#[cfg(test)]
#[path = "service_auth_format_tests.rs"]
mod tests;

use shaula_core::auth::TargetAllowlist;
use shaula_core::auth_policy::{TargetPolicy, AUTH_POLICY_SCHEMA_VERSION};
use shaula_core::error::ReasonCode;
use shaula_core::registry::AuthProfilePut;

/// The parsed publication format of one Auth Candidate.
#[derive(Debug, Clone)]
pub(crate) enum AuthPutFormat {
    /// Legacy single-installation / exact allowlist (schema_version 1).
    Legacy { allowlist_json: String },
    /// Multi-account GitHub App policy (schema_version 2).
    V2 { app_id: String, policy_json: String },
}

impl AuthPutFormat {
    /// Parses and validates the format members off the payload.
    /// Errors map to `422 SpecInvalid` at the HTTP boundary.
    pub(crate) fn parse(payload: &AuthProfilePut) -> Result<Self, (ReasonCode, String)> {
        match payload.schema_version {
            None | Some(1) => Self::parse_legacy(payload),
            Some(v) if v == AUTH_POLICY_SCHEMA_VERSION => Self::parse_v2(payload),
            Some(other) => Err((
                ReasonCode::SpecInvalid,
                format!("unsupported auth schema version {other}"),
            )),
        }
    }

    fn parse_legacy(payload: &AuthProfilePut) -> Result<Self, (ReasonCode, String)> {
        // The v2 policy field is never accepted without its explicit
        // version — payload shape must not select a format.
        if payload.target_policy.is_some() {
            return Err((
                ReasonCode::SpecInvalid,
                "target_policy requires schema_version: 2".into(),
            ));
        }
        // Presence is required: an absent member is a 422 (never an empty
        // allowlist that then fails later with a vaguer message).
        let members = payload.allowlist.clone().ok_or_else(|| {
            (
                ReasonCode::SpecInvalid,
                "target_allowlist is required".into(),
            )
        })?;
        let allowlist = TargetAllowlist::new(members).map_err(|e| (e.code, e.summary))?;
        let allowlist_json =
            serde_json::to_string(&allowlist).map_err(|e| (ReasonCode::Internal, e.to_string()))?;
        Ok(Self::Legacy { allowlist_json })
    }

    fn parse_v2(payload: &AuthProfilePut) -> Result<Self, (ReasonCode, String)> {
        if payload.kind != shaula_core::auth::AuthKind::GithubApp {
            return Err((
                ReasonCode::SpecInvalid,
                "schema_version 2 requires the github_app kind".into(),
            ));
        }
        // v2 never mixes with the legacy single-installation members.
        if payload.installation_id.is_some()
            || payload.pat_identity.is_some()
            // Present regardless of value (even []) = forbidden member.
            || payload.allowlist.is_some()
        {
            return Err((
                ReasonCode::SpecInvalid,
                "schema_version 2 forbids installation_id, pat and target_allowlist fields".into(),
            ));
        }
        let app_id = payload.app_id.as_deref().unwrap_or_default();
        // The App ID is a positive decimal string in the new format.
        if app_id.is_empty()
            || !app_id.bytes().all(|b| b.is_ascii_digit())
            || app_id.starts_with('0')
        {
            return Err((
                ReasonCode::SpecInvalid,
                "schema_version 2 requires a positive decimal app_id".into(),
            ));
        }
        let selectors = payload
            .target_policy
            .clone()
            .ok_or_else(|| (ReasonCode::SpecInvalid, "target_policy is required".into()))?;
        let policy = TargetPolicy::new(selectors).map_err(|e| (e.code, e.summary))?;
        let policy_json =
            serde_json::to_string(&policy).map_err(|e| (ReasonCode::Internal, e.to_string()))?;
        Ok(Self::V2 {
            app_id: app_id.to_string(),
            policy_json,
        })
    }

    /// Canonical body fingerprint for idempotency: covers EVERY non-secret
    /// semantic member, so changed content conflicts instead of replaying.
    /// The LEGACY encoding is preserved BYTE-FOR-BYTE from the baseline
    /// (five unprefixed members): identical retries against persisted
    /// pre-upgrade idempotency entries must replay, never conflict
    /// (spec 0011 §7.1). Only v2 entries are version-prefixed — they
    /// never existed before this format.
    pub(crate) fn canonical_body(
        &self,
        payload: &AuthProfilePut,
    ) -> Result<String, shaula_core::error::CoreError> {
        Ok(match self {
            AuthPutFormat::Legacy { allowlist_json } => format!(
                "{}|{}|{}|{}|{}",
                payload.kind.as_str(),
                payload.app_id.as_deref().unwrap_or(""),
                payload.installation_id.unwrap_or(0),
                payload.pat_identity.as_deref().unwrap_or(""),
                allowlist_json,
            ),
            AuthPutFormat::V2 {
                app_id,
                policy_json,
            } => {
                format!("2|{app_id}|{policy_json}")
            }
        })
    }
}
