//! Forgejo provider identities and Fleet configuration.
//!
//! Forgejo is intentionally modeled separately from GitHub: it has no scale
//! sets, runner groups, installation identity, or route proof.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::auth::validate_profile_key_for_fleet_ref;
use crate::error::{CoreError, CoreResult, ReasonCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgejoScopeKind {
    Instance,
    Organization,
    User,
    Repository,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ForgejoScope {
    Instance,
    Organization { name: String },
    User,
    Repository { owner: String, name: String },
}

impl ForgejoScope {
    pub fn kind(&self) -> ForgejoScopeKind {
        match self {
            Self::Instance => ForgejoScopeKind::Instance,
            Self::Organization { .. } => ForgejoScopeKind::Organization,
            Self::User => ForgejoScopeKind::User,
            Self::Repository { .. } => ForgejoScopeKind::Repository,
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        match self {
            Self::Instance | Self::User => Ok(()),
            Self::Organization { name } => validate_segment(name, "organization name"),
            Self::Repository { owner, name } => {
                validate_segment(owner, "repository owner")?;
                validate_segment(name, "repository name")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgejoTarget {
    pub instance_url: String,
    pub scope: ForgejoScope,
}

/// Non-secret registration data exposed to a template. The one-shot token
/// is deliberately not representable in this Terraform input type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgejoBootstrapIdentity {
    pub instance_url: String,
    pub uuid: String,
    pub labels: Vec<String>,
}

impl ForgejoTarget {
    pub fn validate(&self) -> CoreResult<()> {
        let url = Url::parse(&self.instance_url).map_err(|error| {
            CoreError::new(
                ReasonCode::SpecInvalid,
                format!("invalid Forgejo instance_url: {error}"),
            )
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none_or(|host| host.is_empty())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "instance_url must be an http(s) URL without credentials, query, or fragment",
            ));
        }
        self.scope.validate()
    }

    pub fn scope_key(&self) -> String {
        let scope = match &self.scope {
            ForgejoScope::Instance => "instance".to_string(),
            ForgejoScope::Organization { name } => format!("organization:{name}"),
            ForgejoScope::User => "user".to_string(),
            ForgejoScope::Repository { owner, name } => {
                format!("repository:{owner}/{name}")
            }
        };
        format!("{}|{scope}", self.instance_url)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetForgejoSection {
    pub instance_url: String,
    pub scope: ForgejoScope,
    pub auth_profile_ref: String,
    pub runner_name_prefix: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

impl FleetForgejoSection {
    pub fn target(&self) -> ForgejoTarget {
        ForgejoTarget {
            instance_url: self.instance_url.clone(),
            scope: self.scope.clone(),
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        self.target().validate()?;
        validate_profile_key_for_fleet_ref(&self.auth_profile_ref)?;
        if self.runner_name_prefix.trim().is_empty()
            || self.runner_name_prefix.len() > 100
            || self.runner_name_prefix.chars().any(char::is_control)
        {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "runner_name_prefix must be 1..=100 characters",
            ));
        }
        validate_labels(&self.labels)
    }
}

pub fn validate_labels(labels: &[String]) -> CoreResult<()> {
    if labels.is_empty() || labels.len() > 32 {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "expected 1..=32 Forgejo labels",
        ));
    }
    let mut names = std::collections::BTreeSet::new();
    for label in labels {
        let (name, target) = label
            .split_once(':')
            .map_or((label.as_str(), None), |(name, target)| {
                (name, Some(target))
            });
        if label.trim() != label
            || label.len() > 255
            || name.is_empty()
            || label
                .chars()
                .any(|character| character.is_control() || character == ',')
            || name.trim() != name
            || target.is_some_and(|target| target.trim().is_empty())
            || !names.insert(name)
        {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "Forgejo labels must have distinct nonempty names and valid backend targets",
            ));
        }
    }
    Ok(())
}

/// Releases only: unknown/custom version strings and prereleases do not prove
/// the minimum supported protocol. Build metadata is allowed by SemVer.
pub fn supports_server_version(version: &str) -> bool {
    supports_release(version, 15)
}

pub(crate) fn supports_runner_version(version: &str) -> bool {
    supports_release(version, 13)
}

fn supports_release(version: &str, minimum_major: u64) -> bool {
    semver::Version::parse(version.strip_prefix('v').unwrap_or(version))
        .is_ok_and(|version| version.major >= minimum_major && version.pre.is_empty())
}

fn validate_segment(value: &str, field: &str) -> CoreResult<()> {
    if value.trim().is_empty()
        || value.len() > 255
        || value == "."
        || value == ".."
        || value.chars().any(char::is_control)
    {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            format!("{field} is not a valid path segment"),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn versions_require_a_supported_stable_release() {
        for version in ["15.0.0", "16.0.4", "v16.0.4+build"] {
            assert!(supports_server_version(version));
        }
        for version in ["14.1.2", "15.0.0-rc1", "15", "garbage", "1.26.0"] {
            assert!(!supports_server_version(version));
        }
        assert!(supports_runner_version("13.1.0"));
        assert!(!supports_runner_version("12.7.0"));
        assert!(!supports_runner_version("13.0.0-rc1"));
    }

    #[test]
    fn forgejo_target_validates_scope_and_url() {
        let target = ForgejoTarget {
            instance_url: "https://forgejo.example.test/".into(),
            scope: ForgejoScope::Repository {
                owner: "owner".into(),
                name: "repo".into(),
            },
        };
        assert!(target.validate().is_ok());
        assert!(ForgejoTarget {
            instance_url: "file:///tmp/forgejo".into(),
            scope: ForgejoScope::Instance,
        }
        .validate()
        .is_err());
    }

    #[test]
    fn forgejo_section_rejects_empty_prefix_and_labels() {
        let section = FleetForgejoSection {
            instance_url: "https://forgejo.example.test".into(),
            scope: ForgejoScope::Instance,
            auth_profile_ref: "forgejo-prod".into(),
            runner_name_prefix: "".into(),
            labels: vec!["linux".into()],
        };
        assert!(section.validate().is_err());
    }
}
