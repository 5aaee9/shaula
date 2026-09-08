//! Target selectors and the versioned Target policy of a GitHub App Auth
//! Revision (spec 0011 §2–3). Pure domain: construction validates, matching
//! is ASCII case-insensitive on names while the stored spelling stays the
//! operator's submitted canonical form for display.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ReasonCode};
use crate::github::GitHubTarget;

/// `schema_version` of the multi-account GitHub App publication format.
pub const AUTH_POLICY_SCHEMA_VERSION: i64 = 2;

/// Non-empty policy bounds (spec 0011 §3): at most 100 selectors over at
/// most 50 distinct accounts.
pub const MAX_SELECTORS: usize = 100;
pub const MAX_ACCOUNTS: usize = 50;

/// GitHub account kind addressed by an `account_repositories` selector.
/// Distinguishes a personal account's repositories from an organization's,
/// because the corresponding Fleet Targets and runner scoping differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    User,
    Organization,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::User => "user",
            AccountKind::Organization => "organization",
        }
    }
}

/// One admission rule of a Target policy. Discriminated and strict: no
/// globs, no URLs, no path fragments — names reuse the exact GitHub
/// identifier validation of [`GitHubTarget`], so `*`, `../x` and friends
/// are rejected at the parse boundary instead of entering the domain.
///
/// Matching is by ASCII case-insensitive name equality; GitHub's canonical
/// spelling is a display concern and never rewrites the stored selector.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetSelector {
    /// Admits the organization Fleet Target itself — never its repositories.
    Organization { owner: String },
    /// Admits exactly one repository Fleet Target; a same-name rebuilt
    /// repository is NOT covered by identity, only by this address.
    Repository { owner: String, repository: String },
    /// Admits repository Fleet Targets currently owned by the named account
    /// AND accessible to the App installation. Never the organization Target
    /// itself and never repositories the account merely collaborates on.
    AccountRepositories {
        account_kind: AccountKind,
        owner: String,
    },
}

impl TargetSelector {
    /// The account this selector addresses (used for the distinct-account
    /// budget and for converging one account's selectors onto one
    /// installation).
    pub fn account(&self) -> &str {
        match self {
            TargetSelector::Organization { owner }
            | TargetSelector::Repository { owner, .. }
            | TargetSelector::AccountRepositories { owner, .. } => owner,
        }
    }

    /// Canonical ordering key: kind, account kind, owner, repository —
    /// case-insensitive, so semantically identical order permutations
    /// cannot produce distinct mutations.
    fn sort_key(&self) -> (u8, u8, String, String) {
        let kind_rank = match self {
            TargetSelector::Organization { .. } => 0u8,
            TargetSelector::Repository { .. } => 1u8,
            TargetSelector::AccountRepositories { .. } => 2u8,
        };
        let account_kind_rank = match self {
            TargetSelector::AccountRepositories {
                account_kind: AccountKind::User,
                ..
            } => 0u8,
            TargetSelector::AccountRepositories {
                account_kind: AccountKind::Organization,
                ..
            } => 1u8,
            _ => 0u8,
        };
        let repository = match self {
            TargetSelector::Repository { repository, .. } => repository.to_ascii_lowercase(),
            _ => String::new(),
        };
        (
            kind_rank,
            account_kind_rank,
            self.account().to_ascii_lowercase(),
            repository,
        )
    }

    /// Whether this single selector admits the concrete Fleet Target.
    pub fn allows(&self, target: &GitHubTarget) -> bool {
        match (self, target) {
            (TargetSelector::Organization { owner }, GitHubTarget::Organization { owner: o }) => {
                owner.eq_ignore_ascii_case(o)
            }
            (
                TargetSelector::Repository { owner, repository },
                GitHubTarget::Repository {
                    owner: o,
                    repository: r,
                },
            ) => owner.eq_ignore_ascii_case(o) && repository.eq_ignore_ascii_case(r),
            (
                TargetSelector::AccountRepositories { owner, .. },
                GitHubTarget::Repository { owner: o, .. },
            ) => owner.eq_ignore_ascii_case(o),
            _ => false,
        }
    }
}

impl<'de> Deserialize<'de> for TargetSelector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
        enum Raw {
            Organization {
                owner: String,
            },
            Repository {
                owner: String,
                repository: String,
            },
            AccountRepositories {
                account_kind: AccountKind,
                owner: String,
            },
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(match raw {
            Raw::Organization { owner } => {
                Self::organization(owner).map_err(serde::de::Error::custom)?
            }
            Raw::Repository { owner, repository } => {
                Self::repository(owner, repository).map_err(serde::de::Error::custom)?
            }
            Raw::AccountRepositories {
                account_kind,
                owner,
            } => {
                Self::account_repositories(account_kind, owner).map_err(serde::de::Error::custom)?
            }
        })
    }
}

impl TargetSelector {
    pub fn organization(owner: impl Into<String>) -> CoreResult<Self> {
        // Single validation rule: the exact GitHub owner-name check.
        let validated = GitHubTarget::organization(owner.into())?;
        Ok(TargetSelector::Organization {
            owner: validated.owner().to_string(),
        })
    }

    pub fn repository(owner: impl Into<String>, repository: impl Into<String>) -> CoreResult<Self> {
        let validated = GitHubTarget::new_repository(owner.into(), repository)?;
        match validated {
            GitHubTarget::Repository { owner, repository } => {
                Ok(TargetSelector::Repository { owner, repository })
            }
            GitHubTarget::Organization { .. } => unreachable!("repository constructor input"),
        }
    }

    pub fn account_repositories(
        account_kind: AccountKind,
        owner: impl Into<String>,
    ) -> CoreResult<Self> {
        let validated = GitHubTarget::organization(owner.into())?;
        Ok(TargetSelector::AccountRepositories {
            account_kind,
            owner: validated.owner().to_string(),
        })
    }
}

/// The non-empty, de-duplicated, canonically ordered Target policy frozen
/// in one v2 GitHub App Auth Revision. Order variations of the same set
/// deserialize to the SAME policy, so replay hashes stay stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetPolicy {
    selectors: Vec<TargetSelector>,
}

impl TargetPolicy {
    /// Validates, rejects duplicate selectors and orders canonically.
    pub fn new(mut selectors: Vec<TargetSelector>) -> CoreResult<Self> {
        if selectors.is_empty() {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "target policy must not be empty",
            ));
        }
        if selectors.len() > MAX_SELECTORS {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                format!("target policy exceeds {MAX_SELECTORS} selectors"),
            ));
        }
        selectors.sort_by_key(TargetSelector::sort_key);
        // Case-insensitive duplicate rejection: an exact byte duplicate and
        // a case variant of the same selector are both `422 SpecInvalid`.
        for pair in selectors.windows(2) {
            if pair[0].sort_key() == pair[1].sort_key() {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "target policy contains duplicate selectors",
                ));
            }
        }
        let mut accounts: Vec<&str> = selectors.iter().map(|s| s.account()).collect();
        accounts.sort_by_key(|a| a.to_ascii_lowercase());
        accounts.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        if accounts.len() > MAX_ACCOUNTS {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                format!("target policy exceeds {MAX_ACCOUNTS} distinct accounts"),
            ));
        }
        Ok(Self { selectors })
    }

    pub fn selectors(&self) -> &[TargetSelector] {
        &self.selectors
    }

    /// Every selector admitting the target. Multiple matches are legal only
    /// when they converge on one installation — resolution (spec 0011 §4.2)
    /// merges by identity, never by array order.
    pub fn matches(&self, target: &GitHubTarget) -> Vec<&TargetSelector> {
        self.selectors.iter().filter(|s| s.allows(target)).collect()
    }

    /// Whether any selector admits the target.
    pub fn allows(&self, target: &GitHubTarget) -> bool {
        self.selectors.iter().any(|s| s.allows(target))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "auth_policy_tests.rs"]
mod tests;
