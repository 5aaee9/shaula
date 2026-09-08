//! Verified Account Bindings frozen in an Auth Revision and the exact
//! Resolved Auth Context persisted per Fleet Target (spec 0011 §2, §4.2).
//! Non-secret route metadata only: tokens and PEM never enter these types.

use serde::{Deserialize, Serialize};

use crate::auth_policy::AccountKind;
use crate::github::GitHubTarget;

/// GitHub's installation repository-selection mode. `all` may cover future
/// repositories; `selected` covers only the set GitHub currently selected.
/// The UI must keep the two visually distinct (spec 0011 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositorySelection {
    All,
    Selected,
}

impl RepositorySelection {
    pub fn as_str(self) -> &'static str {
        match self {
            RepositorySelection::All => "all",
            RepositorySelection::Selected => "selected",
        }
    }
}

/// One proven App↔account installation relationship, frozen into the Auth
/// Revision that validated it. Numeric identity is authoritative: a
/// reinstall changes `installation_id` and requires a new Candidate, never
/// a background overwrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountBinding {
    /// GitHub account id (also the organization id for org installations).
    pub account_id: i64,
    pub account_kind: AccountKind,
    /// Canonical login as GitHub returned it at validation time (display).
    pub login: String,
    /// Non-zero installation id proven against the App.
    pub installation_id: i64,
    pub repository_selection: RepositorySelection,
    /// Unix ms when the binding was verified.
    pub validated_at_ms: i64,
}

impl AccountBinding {
    pub fn account_matches(&self, selector_account: &str) -> bool {
        self.login.eq_ignore_ascii_case(selector_account)
    }
}

/// Exact verified route identity for one Fleet Target under one Auth
/// Revision (spec 0011 §4.2). Persisted per fleet; never reconstructible
/// from the profile key or installation id alone. `repository_id` /
/// `repository_owner_id` are pinned on first verified resolution and a
/// same-name rebuild can never silently overwrite them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAuthContext {
    pub profile_key: String,
    pub revision: i64,
    /// Fixed GitHub host the revision's API calls derive from.
    pub github_host: String,
    pub app_id: String,
    pub account_id: i64,
    pub account_kind: AccountKind,
    /// Canonical login as GitHub returned it at validation time (display).
    pub login: String,
    pub installation_id: i64,
    pub target: GitHubTarget,
    /// For organization targets: the organization id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<i64>,
    /// For repository targets: pinned repository and owner ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_owner_id: Option<i64>,
}

impl ResolvedAuthContext {
    /// The complete numeric target identity required before execution.
    pub fn has_complete_identity(&self) -> bool {
        if self.account_id <= 0 || self.installation_id <= 0 || self.github_host != "github.com" {
            return false;
        }
        match self.target {
            GitHubTarget::Organization { .. } => {
                self.account_kind == AccountKind::Organization
                    && self.organization_id == Some(self.account_id)
                    && self.repository_id.is_none()
                    && self.repository_owner_id.is_none()
            }
            GitHubTarget::Repository { .. } => {
                self.repository_id.is_some_and(|id| id > 0)
                    && self.repository_owner_id == Some(self.account_id)
                    && self.organization_id.is_none()
            }
        }
    }

    /// A verified completion may fill absent pins, but never change route
    /// authority or an identity already proven at admission.
    pub fn completes(&self, desired: &Self) -> bool {
        self.has_complete_identity()
            && self.profile_key == desired.profile_key
            && self.revision == desired.revision
            && self.github_host == desired.github_host
            && self.app_id == desired.app_id
            && self.target == desired.target
            && self.account_id == desired.account_id
            && self.account_kind == desired.account_kind
            && self.installation_id == desired.installation_id
            && crate::registry::auth_context_pins_agree(desired, self)
    }

    /// Whether the context still matches the exact Auth Revision Ref and
    /// the binding identity it was resolved under. Used by the handoff CAS:
    /// an equal ref tuple with a drifted context is NOT the same route.
    pub fn matches_ref_and_binding(
        &self,
        profile_key: &str,
        revision: i64,
        binding: &AccountBinding,
    ) -> bool {
        self.profile_key == profile_key
            && self.revision == revision
            && self.account_id == binding.account_id
            && self.account_kind == binding.account_kind
            && self.installation_id == binding.installation_id
    }
}

/// Bounded route-proof freshness windows (spec 0011 §5.3): positive
/// authorization evidence is reused for at most 60 seconds, negative
/// results for at most 15 seconds. Never persisted as indefinite grants.
pub const POSITIVE_PROOF_TTL_MS: i64 = 60_000;
pub const NEGATIVE_PROOF_TTL_MS: i64 = 15_000;

/// The local structural resolution of one Fleet Target against a v2
/// revision's policy and frozen bindings (spec 0011 §4.2 steps 1–2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesiredContextResolution {
    /// Exactly one route identity: the desired context intent.
    Resolved(Box<ResolvedAuthContext>),
    /// No selector admits the target (`TargetNotAllowed`).
    NoMatchingSelector,
    /// Matching selectors converge on different installations for the
    /// same target (`AmbiguousInstallation`) — never resolved by order.
    AmbiguousInstallation,
}

/// Derives the desired Resolved Auth Context for a concrete target. Pure
/// and offline: the result records intent, not GitHub access, which the
/// asynchronous reconcile must still verify before any remote mutation.
pub fn resolve_desired_context(
    profile_key: &str,
    revision: i64,
    app_id: &str,
    policy: &crate::auth_policy::TargetPolicy,
    bindings: &[AccountBinding],
    target: &GitHubTarget,
) -> DesiredContextResolution {
    let matches = policy.matches(target);
    if matches.is_empty() {
        return DesiredContextResolution::NoMatchingSelector;
    }
    let mut route: Option<(i64, i64, AccountKind, String)> = None;
    for selector in matches {
        let Some(binding) = bindings
            .iter()
            .find(|b| b.account_matches(selector.account()))
        else {
            // A selector without a frozen binding cannot route; the
            // revision is corrupt (validation always freezes one binding
            // per selector) — refuse rather than guess.
            return DesiredContextResolution::NoMatchingSelector;
        };
        match &route {
            None => {
                route = Some((
                    binding.account_id,
                    binding.installation_id,
                    binding.account_kind,
                    binding.login.clone(),
                ));
            }
            Some((account_id, installation_id, _, _)) => {
                // Equivalent matches merge ONLY on identical route identity.
                if *account_id != binding.account_id || *installation_id != binding.installation_id
                {
                    return DesiredContextResolution::AmbiguousInstallation;
                }
            }
        }
    }
    let Some((account_id, installation_id, account_kind, login)) = route else {
        return DesiredContextResolution::NoMatchingSelector;
    };
    let organization_id = match target {
        GitHubTarget::Organization { .. } => Some(account_id),
        GitHubTarget::Repository { .. } => None,
    };
    DesiredContextResolution::Resolved(Box::new(ResolvedAuthContext {
        profile_key: profile_key.to_string(),
        revision,
        github_host: "github.com".to_string(),
        app_id: app_id.to_string(),
        account_id,
        account_kind,
        installation_id,
        login,
        target: target.clone(),
        organization_id,
        repository_id: None,
        repository_owner_id: None,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn repository_selection_wire_names() {
        assert_eq!(
            serde_json::to_string(&RepositorySelection::All).unwrap(),
            r#""all""#
        );
        assert_eq!(
            serde_json::to_string(&RepositorySelection::Selected).unwrap(),
            r#""selected""#
        );
    }

    #[test]
    fn context_ref_and_binding_match_requires_full_identity() {
        let binding = AccountBinding {
            account_id: 7,
            account_kind: AccountKind::User,
            login: "5aaee9".into(),
            installation_id: 42,
            repository_selection: RepositorySelection::All,
            validated_at_ms: 1,
        };
        let context = ResolvedAuthContext {
            profile_key: "shared".into(),
            revision: 3,
            github_host: "github.com".into(),
            app_id: "4863460".into(),
            account_id: 7,
            account_kind: AccountKind::User,
            login: "5aaee9".into(),
            installation_id: 42,
            target: GitHubTarget::new_repository("5aaee9", "proj").unwrap(),
            organization_id: None,
            repository_id: Some(100),
            repository_owner_id: Some(7),
        };
        assert!(context.matches_ref_and_binding("shared", 3, &binding));
        // A reinstall (new installation id) is a different route even with
        // an identical ref tuple.
        let reinstalled = AccountBinding {
            installation_id: 43,
            ..binding.clone()
        };
        assert!(!context.matches_ref_and_binding("shared", 3, &reinstalled));
        assert!(!context.matches_ref_and_binding("shared", 4, &binding));
        assert!(!context.matches_ref_and_binding("other", 3, &binding));
    }

    #[test]
    fn context_serialization_omits_absent_ids() {
        let context = ResolvedAuthContext {
            profile_key: "p".into(),
            revision: 1,
            github_host: "github.com".into(),
            app_id: "1".into(),
            account_id: 2,
            account_kind: AccountKind::Organization,
            login: "o".into(),
            installation_id: 3,
            target: GitHubTarget::organization("o").unwrap(),
            organization_id: Some(2),
            repository_id: None,
            repository_owner_id: None,
        };
        let json = serde_json::to_string(&context).unwrap();
        assert!(!json.contains("repository_id"));
        assert!(json.contains("organization_id"));
        let round: ResolvedAuthContext = serde_json::from_str(&json).unwrap();
        assert_eq!(round, context);
    }
}
