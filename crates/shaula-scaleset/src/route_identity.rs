//! Identity comparisons for runtime authorization; remote data never replaces a pin.

use shaula_core::auth_policy::AccountKind;
use shaula_core::github::GitHubTarget;
use shaula_core::ports::{AccessFailure, TargetIdentity};

use crate::installation::InstallationProof;
use crate::ScalesetClient;

pub(crate) const MAX_GITHUB_ID: i64 = (1_i64 << 53) - 1;

pub(crate) fn valid_id(id: i64) -> bool {
    (1..=MAX_GITHUB_ID).contains(&id)
}

impl ScalesetClient {
    pub(crate) fn verify_route_identity(
        &self,
        installation: &InstallationProof,
        identity: &TargetIdentity,
    ) -> Result<(), AccessFailure> {
        let owner_matches = match &self.config.target {
            GitHubTarget::Organization { .. } => {
                installation.account_kind == AccountKind::Organization
                    && identity.organization_id == Some(installation.account_id)
            }
            GitHubTarget::Repository { .. } => {
                identity.repository_owner_id == Some(installation.account_id)
            }
        };
        if !owner_matches
            || !installation
                .login
                .eq_ignore_ascii_case(self.config.target.owner())
        {
            return Err(AccessFailure::PermissionDenied);
        }
        if let Some(expected) = &self.expected_context {
            if expected.profile_key.is_empty()
                || expected.revision <= 0
                || expected.github_host != "github.com"
                || expected.target != self.config.target
                || expected.app_id != installation.app_id.to_string()
                || expected.installation_id != installation.installation_id
                || expected.account_id != installation.account_id
                || expected.account_kind != installation.account_kind
                || !expected.login.eq_ignore_ascii_case(&installation.login)
                || expected.organization_id != identity.organization_id
                || expected.repository_id != identity.repository_id
                || expected.repository_owner_id != identity.repository_owner_id
            {
                return Err(AccessFailure::PermissionDenied);
            }
        }
        Ok(())
    }
}

pub(crate) fn parse_target_identity(
    target: &GitHubTarget,
    body: &serde_json::Value,
) -> Result<TargetIdentity, AccessFailure> {
    let id = numeric_id(body.get("id"))?;
    match target {
        GitHubTarget::Organization { owner } => {
            if !matches_name(body.get("login"), owner) {
                return Err(AccessFailure::PermissionDenied);
            }
            Ok(TargetIdentity {
                organization_id: Some(id),
                repository_id: None,
                repository_owner_id: None,
            })
        }
        GitHubTarget::Repository { owner, repository } => {
            let owner_id = numeric_id(body.pointer("/owner/id"))?;
            if !matches_name(body.pointer("/owner/login"), owner)
                || !matches_name(body.get("name"), repository)
            {
                return Err(AccessFailure::PermissionDenied);
            }
            Ok(TargetIdentity {
                organization_id: None,
                repository_id: Some(id),
                repository_owner_id: Some(owner_id),
            })
        }
    }
}

fn numeric_id(value: Option<&serde_json::Value>) -> Result<i64, AccessFailure> {
    value
        .and_then(serde_json::Value::as_i64)
        .filter(|id| valid_id(*id))
        .ok_or_else(invalid_identity)
}

fn matches_name(value: Option<&serde_json::Value>, expected: &str) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

pub(crate) fn invalid_identity() -> AccessFailure {
    AccessFailure::Unavailable {
        summary: "target identity response missing complete legal numeric identity".into(),
    }
}
