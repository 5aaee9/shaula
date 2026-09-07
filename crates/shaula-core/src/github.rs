//! Typed GitHub target and scale set identity.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ReasonCode};

/// GitHub names are normalized: strict identifier form, no path fragments.
const NAME_PATTERN: &str = r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38}[A-Za-z0-9])?$|^[A-Za-z0-9]$";
/// Repository names allow dots additionally.
const REPO_PATTERN: &str = r"^[A-Za-z0-9._-]{1,100}$";

/// A Scale Set registration destination on `github.com`. Exactly one shape is
/// accepted; callers never submit URLs or generic endpoints.
///
/// Deserialization is VALIDATED: it goes through the same normalization
/// constructors as programmatic construction, so `owner='../etc'`, an
/// unexpected `repository` field on an organization, or any other
/// non-normalized value can never enter the domain through the serde
/// boundary (api-parse-dont-validate; single validation rule).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitHubTarget {
    Organization { owner: String },
    Repository { owner: String, repository: String },
}

impl<'de> Deserialize<'de> for GitHubTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
        enum Raw {
            Organization { owner: String },
            Repository { owner: String, repository: String },
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(match raw {
            Raw::Organization { owner } => {
                Self::organization(owner).map_err(serde::de::Error::custom)?
            }
            Raw::Repository { owner, repository } => {
                Self::new_repository(owner, repository).map_err(serde::de::Error::custom)?
            }
        })
    }
}

impl GitHubTarget {
    pub fn organization(owner: impl Into<String>) -> CoreResult<Self> {
        let owner = normalize_name(&owner.into())?;
        Ok(Self::Organization { owner })
    }

    pub fn new_repository(
        owner: impl Into<String>,
        repository: impl Into<String>,
    ) -> CoreResult<Self> {
        let owner = normalize_name(&owner.into())?;
        let repository = normalize_repo(&repository.into())?;
        Ok(Self::Repository { owner, repository })
    }

    pub fn owner(&self) -> &str {
        match self {
            GitHubTarget::Organization { owner } => owner,
            GitHubTarget::Repository { owner, .. } => owner,
        }
    }

    pub fn repository_name(&self) -> Option<&str> {
        match self {
            GitHubTarget::Organization { .. } => None,
            GitHubTarget::Repository { repository, .. } => Some(repository),
        }
    }

    /// The canonical `github.com` configuration URL derived internally.
    /// Caller URLs are never used as generic endpoints.
    pub fn config_url(&self) -> String {
        match self {
            GitHubTarget::Organization { owner } => {
                format!("https://github.com/{owner}")
            }
            GitHubTarget::Repository { owner, repository } => {
                format!("https://github.com/{owner}/{repository}")
            }
        }
    }
}

fn normalize_name(value: &str) -> CoreResult<String> {
    let value = value.trim();
    let re = regex::Regex::new(NAME_PATTERN)
        .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
    if re.is_match(value) && value.ne("enterprises") {
        Ok(value.to_string())
    } else {
        Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "github owner name is not a normalized identifier",
        ))
    }
}

fn normalize_repo(value: &str) -> CoreResult<String> {
    let value = value.trim();
    let re = regex::Regex::new(REPO_PATTERN)
        .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
    if re.is_match(value) && !value.starts_with('.') {
        Ok(value.to_string())
    } else {
        Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "github repository name is not a normalized identifier",
        ))
    }
}

/// Remote scale set identity: target + runner group + scale set name.
/// This tuple is fixed for a Fleet incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScaleSetIdentity {
    pub target: GitHubTarget,
    pub runner_group: String,
    pub scale_set_name: String,
}

/// Runner labels; each bounded in length and count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(rename = "type")]
    pub label_type: String,
}

impl Label {
    pub fn system(name: impl Into<String>) -> CoreResult<Self> {
        Ok(Self {
            name: name.into(),
            label_type: "System".to_string(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn deserialization_is_validated_at_the_boundary() {
        // A normalized value deserializes fine.
        let target: GitHubTarget =
            serde_json::from_str(r#"{"kind":"organization","owner":"example-org"}"#).unwrap();
        assert_eq!(target.owner(), "example-org");

        // Path fragments are rejected, not silently accepted.
        assert!(serde_json::from_str::<GitHubTarget>(
            r#"{"kind":"organization","owner":"../etc"}"#
        )
        .is_err());
        // An organization carrying a repository field cannot smuggle it
        // past the exactly-one-shape contract.
        assert!(serde_json::from_str::<GitHubTarget>(
            r#"{"kind":"organization","owner":"example-org","repository":"repo"}"#
        )
        .is_err());
    }

    #[test]
    fn organization_target_round_trip() {
        let target = GitHubTarget::organization("Example-Org").unwrap();
        assert_eq!(target.owner(), "Example-Org");
        assert_eq!(target.config_url(), "https://github.com/Example-Org");
    }

    #[test]
    fn repository_target_round_trip() {
        let target = GitHubTarget::new_repository("example-org", "example-repo").unwrap();
        assert_eq!(
            target.config_url(),
            "https://github.com/example-org/example-repo"
        );
    }

    #[test]
    fn malformed_names_fail_admission() {
        assert!(GitHubTarget::organization("has space").is_err());
        assert!(GitHubTarget::organization("../etc").is_err());
        assert!(GitHubTarget::organization("").is_err());
        assert!(GitHubTarget::new_repository("ok-org", "a/b").is_err());
        assert!(GitHubTarget::new_repository("ok-org", ".hidden").is_err());
    }

    #[test]
    fn enterpises_reserved_as_owner() {
        // "enterprises" would be misread as a repository path by the
        // protocol; keep it reserved.
        assert!(GitHubTarget::organization("enterprises").is_err());
    }
}
