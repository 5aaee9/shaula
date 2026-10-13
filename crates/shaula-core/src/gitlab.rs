//! GitLab provider identities and Fleet configuration (spec 0043).
//!
//! GitLab is modeled separately from GitHub and Forgejo: demand comes from a
//! Fleet-owned paused anchor registration, runner tokens are reusable, and a
//! deleted registration does not interrupt its job.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::auth::validate_profile_key_for_fleet_ref;
use crate::error::{CoreError, CoreResult, ReasonCode};

/// Instance scope is reserved until R5 freezes; it is not representable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GitlabScope {
    Group { path: String },
    Project { path: String },
}

impl GitlabScope {
    pub fn path(&self) -> &str {
        match self {
            Self::Group { path } | Self::Project { path } => path,
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        let path = self.path();
        if path.trim() != path
            || path.is_empty()
            || path.len() > 255
            || path.starts_with('/')
            || path.ends_with('/')
            || path
                .split('/')
                .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
            || path.chars().any(char::is_control)
            || (matches!(self, Self::Project { .. }) && !path.contains('/'))
        {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "GitLab scope path must be a full group or namespace/project path",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitlabTarget {
    pub instance_url: String,
    pub scope: GitlabScope,
}

impl GitlabTarget {
    pub fn validate(&self) -> CoreResult<()> {
        let url = Url::parse(&self.instance_url).map_err(|error| {
            CoreError::new(
                ReasonCode::SpecInvalid,
                format!("invalid GitLab instance_url: {error}"),
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
            GitlabScope::Group { path } => format!("group:{path}"),
            GitlabScope::Project { path } => format!("project:{path}"),
        };
        format!("{}|{scope}", self.instance_url)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetGitlabSection {
    pub instance_url: String,
    pub scope: GitlabScope,
    pub auth_profile_ref: String,
    pub runner_name_prefix: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub run_untagged: bool,
}

impl FleetGitlabSection {
    pub fn target(&self) -> GitlabTarget {
        GitlabTarget {
            instance_url: self.instance_url.clone(),
            scope: self.scope.clone(),
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        self.target().validate()?;
        validate_profile_key_for_fleet_ref(&self.auth_profile_ref)?;
        if self.runner_name_prefix.trim() != self.runner_name_prefix
            || self.runner_name_prefix.is_empty()
            || self.runner_name_prefix.len() > 100
            || self.runner_name_prefix.chars().any(char::is_control)
        {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "runner_name_prefix must be 1..=100 characters",
            ));
        }
        validate_tags(&self.tags)?;
        if self.tags.is_empty() && !self.run_untagged {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "a GitLab Fleet needs at least one tag or run_untagged",
            ));
        }
        Ok(())
    }

    /// The registration attributes every Generation and the anchor share.
    pub fn matching(&self) -> GitlabMatching {
        let mut tags = self.tags.clone();
        tags.sort();
        GitlabMatching {
            tags,
            run_untagged: self.run_untagged,
        }
    }
}

/// Server-side job matching attributes; tags are kept sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabMatching {
    pub tags: Vec<String>,
    pub run_untagged: bool,
}

impl GitlabMatching {
    pub fn matches(&self, tags: &[String], run_untagged: bool) -> bool {
        let mut observed = tags.to_vec();
        observed.sort();
        observed == self.tags && run_untagged == self.run_untagged
    }
}

pub fn validate_tags(tags: &[String]) -> CoreResult<()> {
    if tags.len() > 32 {
        return Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "expected at most 32 GitLab tags",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for tag in tags {
        if tag.trim() != tag
            || tag.is_empty()
            || tag.len() > 255
            || tag
                .chars()
                .any(|character| character.is_control() || character == ',')
            || !seen.insert(tag)
        {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "GitLab tags must be distinct, nonempty and contain no commas",
            ));
        }
    }
    Ok(())
}

/// Role of a Shaula-created registration, recorded in `maintenance_note`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitlabRegistrationRole {
    Anchor,
    Generation,
}

/// Ownership marker written to and read back from `maintenance_note`.
/// GitLab accepts duplicate descriptions, so the marker is joint evidence.
pub fn ownership_note(fleet: &str, role: GitlabRegistrationRole, name: &str) -> String {
    let role = match role {
        GitlabRegistrationRole::Anchor => "anchor",
        GitlabRegistrationRole::Generation => "generation",
    };
    format!("shaula:fleet={fleet};role={role};name={name}")
}

/// Releases only, GitLab-style `19.4.2` or `19.4.2-ee`. Only 19.4 is verified;
/// the 19.0 floor is provisional until R5 freezes.
pub fn supports_server_version(version: &str) -> bool {
    let core = version.split_once('-').map_or(version, |(core, edition)| {
        if matches!(edition, "ee" | "ce") {
            core
        } else {
            ""
        }
    });
    semver::Version::parse(core).is_ok_and(|version| version.major >= 19 && version.pre.is_empty())
}

/// Official `gitlab/gitlab-runner:vX.Y.Z` releases. `run-single` with
/// `--max-builds` is only verified on 19.4; the 19.0 floor is provisional.
pub(crate) fn supports_runner_version(tag: &str) -> bool {
    tag.strip_prefix('v')
        .and_then(|version| semver::Version::parse(version).ok())
        .is_some_and(|version| version.major >= 19 && version.pre.is_empty())
}

/// `run-single --wait-timeout` for every template. It equals the Pool idle
/// deadline: the process outlives Idle retirement, never the other way round.
pub const RUNNER_WAIT_TIMEOUT_SECS: u64 = 600;

/// Non-secret registration data exposed to a template. The runner token is
/// deliberately not representable in this Terraform input type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitlabBootstrapIdentity {
    pub instance_url: String,
    pub runner_id: u64,
    pub runner_name: String,
}
