//! Fleet desired spec: canonical representation, normalization and
//! admission-time resolution facts (spec 0002 section 4).

use serde::{Deserialize, Serialize};

use crate::auth::AuthRevisionRef;
use crate::capacity::CapacityPolicy;
use crate::error::{CoreError, CoreResult, ReasonCode};
pub use crate::forgejo::FleetForgejoSection;
use crate::forgejo::ForgejoTarget;
use crate::github::{GitHubTarget, Label, ScaleSetIdentity};
pub use crate::gitlab::FleetGitlabSection;
use crate::gitlab::GitlabTarget;
use crate::template::TemplateProfileKey;
pub use crate::template_pool::{
    PoolFailurePolicy, ResolvedTemplatePoolMember, TemplatePoolMember, TemplatePoolSpec,
};

/// Client-submitted Fleet desired spec. Strict JSON rejects unknown fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSpec {
    /// Omitted on legacy GitHub specs; Forgejo specs set this to `forgejo`.
    #[serde(default, skip_serializing_if = "FleetProviderKind::is_github")]
    pub kind: FleetProviderKind,
    /// Forgejo requests may omit the legacy GitHub section. The placeholder
    /// is never admitted and is omitted again during canonical serialization.
    #[serde(default, skip_serializing_if = "is_placeholder_github")]
    pub github: FleetGithubSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forgejo: Option<FleetForgejoSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gitlab: Option<FleetGitlabSection>,
    pub capacity: CapacityPolicyDto,
    #[serde(default)]
    pub template_profile_ref: TemplateProfileRefDto,
    /// Spec 0029 inline pool. Retained for fleets already admitted under it;
    /// new fleets reference a shared pool via `template_pool_ref` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_pool: Option<TemplatePoolSpec>,
    /// Spec 0037: bare key of a shared TemplatePool resource. Mutually
    /// exclusive with `template_profile_ref` and the inline `template_pool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_pool_ref: Option<String>,
    /// Bounded template inputs; schema-checked against the revision the new
    /// Fleet Revision resolves to (the profile's current Active when the
    /// inputs change, else the retained pin).
    #[serde(default)]
    pub template_inputs: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetGithubSection {
    pub target: GitHubTarget,
    pub auth_profile_ref: String,
    pub scale_set_name: String,
    pub runner_group: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FleetProviderKind {
    #[default]
    Github,
    Forgejo,
    Gitlab,
}

impl FleetProviderKind {
    fn is_github(&self) -> bool {
        matches!(self, Self::Github)
    }

    /// Pool backends pre-register runners and have no scale-set session,
    /// Auth Handoff or installation context.
    pub fn is_pool(&self) -> bool {
        match self {
            Self::Github => false,
            Self::Forgejo | Self::Gitlab => true,
        }
    }
}

impl FleetSpec {
    pub fn auth_profile_ref(&self) -> &str {
        match self.kind {
            FleetProviderKind::Github => &self.github.auth_profile_ref,
            FleetProviderKind::Forgejo => self
                .forgejo
                .as_ref()
                .map_or("", |forgejo| forgejo.auth_profile_ref.as_str()),
            FleetProviderKind::Gitlab => self
                .gitlab
                .as_ref()
                .map_or("", |gitlab| gitlab.auth_profile_ref.as_str()),
        }
    }
}

impl Default for FleetGithubSection {
    fn default() -> Self {
        Self {
            target: GitHubTarget::Organization {
                owner: String::new(),
            },
            auth_profile_ref: String::new(),
            scale_set_name: String::new(),
            runner_group: String::new(),
            labels: Vec::new(),
        }
    }
}

fn is_placeholder_github(section: &FleetGithubSection) -> bool {
    section == &FleetGithubSection::default()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityPolicyDto {
    pub min_runners: i64,
    pub max_runners: i64,
}

impl From<CapacityPolicyDto> for CapacityPolicy {
    fn from(dto: CapacityPolicyDto) -> Self {
        CapacityPolicy {
            min_runners: dto.min_runners,
            max_runners: dto.max_runners,
        }
    }
}

/// Bare template profile key. Fleets always follow the profile's latest
/// Active revision (spec 0023) — there is no pinned form. Legacy
/// `{key, revision}` objects stored before ARD-0029 deserialize to their
/// key; the resolved pin lives on the fleet revision row, never here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(transparent)]
pub struct TemplateProfileRefDto(String);

impl TemplateProfileRefDto {
    pub fn key(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for TemplateProfileRefDto {
    fn from(key: String) -> Self {
        Self(key)
    }
}

impl<'de> Deserialize<'de> for TemplateProfileRefDto {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Bare(String),
            // The legacy pin object's revision is deliberately ignored:
            // extra fields are not denied, and the resolved pin authority
            // is the fleet revision row, never this request field.
            Legacy { key: String },
        }
        Ok(match Repr::deserialize(deserializer)? {
            Repr::Bare(key) | Repr::Legacy { key } => Self(key),
        })
    }
}

/// Admission facts frozen into the immutable Fleet Revision. Resolved in the
/// admission transaction; later profile promotions never change a Fleet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetResolvedDependencies {
    pub template_profile: TemplateProfileKey,
    pub template_revision: u64,
    pub template_artifact_digest: String,
    pub template_attestation_id: String,
    /// Admission-time resolved full auth tuple.
    pub auth_desired: AuthRevisionRef,
}

/// Normalized immutable remote identity derived from the spec at admission.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedFleet {
    pub provider: FleetProviderKind,
    pub identity: Option<ScaleSetIdentity>,
    pub forgejo_target: Option<ForgejoTarget>,
    pub gitlab_target: Option<GitlabTarget>,
    pub labels: Vec<Label>,
    pub capacity: CapacityPolicy,
    pub inputs_digest: String,
}

#[path = "fleet_normalize.rs"]
mod normalize;
pub use normalize::{normalize_fleet, validate_fleet_spec};

/// Stable Fleet Key identifying one incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FleetKey(pub String);

impl FleetKey {
    /// Strict parse (R8-01): the shared stable-identifier predicate, no
    /// normalization — the validated bytes are exactly the stored bytes.
    pub fn new(value: impl Into<String>) -> CoreResult<Self> {
        let value = value.into();
        if !crate::auth::is_stable_identifier(&value) {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "fleet key is not a stable identifier",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "fleet_tests.rs"]
mod tests;

/// Parameter object for appending a fleet revision; keeps call sites
/// self-describing instead of positional.
#[derive(Debug, Clone)]
pub struct FleetRevisionInsert {
    pub key: String,
    pub incarnation: String,
    pub revision: i64,
    pub spec_json: String,
    pub template: Option<(String, i64, String, String)>,
    pub template_pool: Vec<crate::template_pool::ResolvedTemplatePoolMember>,
    /// Shared-pool routing context (spec 0037 §4): (pool key, pool
    /// revision) frozen at admission. When set, `template_pool` stays
    /// empty — member rows live on the pool revision, not the fleet.
    pub template_pool_ref: Option<crate::template_pool::FleetPoolRef>,
    pub auth_desired: (String, i64),
    pub inputs_digest: String,
    pub actor: String,
    pub now: i64,
}
