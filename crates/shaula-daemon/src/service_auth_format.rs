//! Validation of the supported GitHub App policy publication format.

#[cfg(test)]
#[path = "service_auth_format_tests.rs"]
mod tests;

use shaula_core::auth_policy::{TargetPolicy, AUTH_POLICY_SCHEMA_VERSION};
use shaula_core::error::ReasonCode;
use shaula_core::registry::AuthProfilePut;

/// Canonical non-secret publication body of a Forgejo token Candidate.
#[derive(Debug, Clone)]
pub(crate) struct ForgejoAuthPutFormat {
    pub target: shaula_core::forgejo::ForgejoTarget,
    pub target_json: String,
}

impl ForgejoAuthPutFormat {
    pub(crate) fn parse(payload: &AuthProfilePut) -> Result<Self, (ReasonCode, String)> {
        if payload.kind != shaula_core::auth::AuthKind::ForgejoToken {
            return Err((
                ReasonCode::SpecInvalid,
                "forgejo_token authentication is required".into(),
            ));
        }
        if payload.schema_version != Some(1) {
            return Err((
                ReasonCode::SpecInvalid,
                "forgejo_token requires schema_version: 1".into(),
            ));
        }
        let Some(target) = payload.forgejo_target.clone() else {
            return Err((
                ReasonCode::SpecInvalid,
                "forgejo_token target is required".into(),
            ));
        };
        target
            .validate()
            .map_err(|error| (error.code, error.summary))?;
        if payload.app_id.is_some() || payload.target_policy.is_some() {
            return Err((
                ReasonCode::SpecInvalid,
                "forgejo_token cannot carry GitHub identity or policy fields".into(),
            ));
        }
        if payload.secret.expose().trim().is_empty() {
            return Err((
                ReasonCode::CredentialMalformed,
                "Forgejo token must not be empty".into(),
            ));
        }
        let target_json = serde_json::to_string(&target)
            .map_err(|error| (ReasonCode::Internal, error.to_string()))?;
        Ok(Self {
            target,
            target_json,
        })
    }

    pub(crate) fn canonical_body(&self) -> String {
        format!("forgejo_token|1|{}", self.target_json)
    }
}

/// Canonical non-secret publication body of one Auth Candidate.
#[derive(Debug, Clone)]
pub(crate) struct AuthPutFormat {
    pub app_id: String,
    pub policy_json: String,
}

impl AuthPutFormat {
    /// Old or unknown formats are rejected before idempotent replay or writes.
    pub(crate) fn parse(payload: &AuthProfilePut) -> Result<Self, (ReasonCode, String)> {
        if payload.schema_version != Some(AUTH_POLICY_SCHEMA_VERSION) {
            return Err((
                ReasonCode::SpecInvalid,
                "schema_version: 2 is required".into(),
            ));
        }
        if payload.kind != shaula_core::auth::AuthKind::GithubApp {
            return Err((
                ReasonCode::SpecInvalid,
                "only github_app authentication is supported".into(),
            ));
        }
        let app_id = payload.app_id.as_deref().unwrap_or_default();
        if app_id.is_empty()
            || !app_id.bytes().all(|b| b.is_ascii_digit())
            || app_id.starts_with('0')
        {
            return Err((
                ReasonCode::SpecInvalid,
                "a positive decimal app_id is required".into(),
            ));
        }
        let selectors = payload
            .target_policy
            .clone()
            .ok_or_else(|| (ReasonCode::SpecInvalid, "target_policy is required".into()))?;
        let policy = TargetPolicy::new(selectors).map_err(|e| (e.code, e.summary))?;
        let policy_json =
            serde_json::to_string(&policy).map_err(|e| (ReasonCode::Internal, e.to_string()))?;
        Ok(Self {
            app_id: app_id.to_string(),
            policy_json,
        })
    }

    /// Versioned fingerprint covers every non-secret semantic field.
    pub(crate) fn canonical_body(&self) -> String {
        format!("2|{}|{}", self.app_id, self.policy_json)
    }
}
