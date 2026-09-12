//! Fleet desired spec: canonical representation, normalization and
//! admission-time resolution facts (spec 0002 section 4).

use serde::{Deserialize, Serialize};

use crate::auth::validate_profile_key_for_fleet_ref;
use crate::auth::AuthRevisionRef;
use crate::capacity::CapacityPolicy;
use crate::error::{CoreError, CoreResult, ReasonCode};
pub use crate::forgejo::FleetForgejoSection;
use crate::forgejo::{validate_labels as validate_forgejo_labels, ForgejoTarget};
use crate::github::{GitHubTarget, Label, ScaleSetIdentity};
use crate::template::TemplateProfileKey;

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
    pub capacity: CapacityPolicyDto,
    pub template_profile_ref: TemplateProfileRefDto,
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
}

impl FleetProviderKind {
    fn is_github(&self) -> bool {
        matches!(self, Self::Github)
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TemplateProfileRefDto(String);

impl TemplateProfileRefDto {
    pub fn key(&self) -> &str {
        &self.0
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
    pub labels: Vec<Label>,
    pub capacity: CapacityPolicy,
    pub inputs_digest: String,
}

/// Canonical validation shared by create, replace and decommission paths.
pub fn validate_fleet_spec(spec: &FleetSpec) -> CoreResult<()> {
    match spec.kind {
        FleetProviderKind::Github => {
            if spec.forgejo.is_some() {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "github fleets must not declare a forgejo section",
                ));
            }
            if spec.github.scale_set_name.trim().is_empty()
                || spec.github.scale_set_name.len() > 100
            {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "scale_set_name must be 1..=100 characters",
                ));
            }
            if spec.github.runner_group.trim().is_empty() || spec.github.runner_group.len() > 100 {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "runner_group must be 1..=100 characters",
                ));
            }
            validate_labels(&spec.github.labels)?;
            validate_profile_key_for_fleet_ref(&spec.github.auth_profile_ref)?;
        }
        FleetProviderKind::Forgejo => {
            if !is_placeholder_github(&spec.github) {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "forgejo fleets must not declare a github section",
                ));
            }
            let Some(forgejo) = &spec.forgejo else {
                return Err(CoreError::new(
                    ReasonCode::SpecInvalid,
                    "forgejo fleets must declare a forgejo section",
                ));
            };
            forgejo.validate()?;
            validate_forgejo_labels(&forgejo.labels)?;
        }
    }
    CapacityPolicy::from(spec.capacity)
        .validate()
        .map_err(|e| CoreError::new(ReasonCode::SpecInvalid, e))?;
    Ok(())
}

fn validate_labels(labels: &[String]) -> CoreResult<()> {
    if labels.len() > 32 {
        return Err(CoreError::new(ReasonCode::SpecInvalid, "at most 32 labels"));
    }
    for label in labels {
        let label = label.trim();
        if label.is_empty() || label.len() > 64 {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "each label must be 1..=64 characters",
            ));
        }
    }
    Ok(())
}

/// Normalizes a validated spec into its immutable identity parts.
pub fn normalize_fleet(spec: &FleetSpec, inputs_digest: String) -> CoreResult<NormalizedFleet> {
    validate_fleet_spec(spec)?;
    let (identity, forgejo_target, labels) = match spec.kind {
        FleetProviderKind::Github => (
            Some(ScaleSetIdentity {
                target: spec.github.target.clone(),
                runner_group: spec.github.runner_group.trim().to_string(),
                scale_set_name: spec.github.scale_set_name.trim().to_string(),
            }),
            None,
            spec.github.labels.clone(),
        ),
        FleetProviderKind::Forgejo => {
            let forgejo = spec.forgejo.as_ref().ok_or_else(|| {
                CoreError::new(ReasonCode::SpecInvalid, "forgejo section is required")
            })?;
            (None, Some(forgejo.target()), forgejo.labels.clone())
        }
    };
    Ok(NormalizedFleet {
        provider: spec.kind,
        identity,
        forgejo_target,
        labels: labels
            .iter()
            .map(|label| Label::system(label.trim()))
            .collect::<CoreResult<Vec<_>>>()?,
        capacity: CapacityPolicy::from(spec.capacity),
        inputs_digest,
    })
}

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
mod tests {
    use super::*;

    fn base_spec() -> FleetSpec {
        FleetSpec {
            kind: FleetProviderKind::Github,
            github: FleetGithubSection {
                target: GitHubTarget::organization("example-org").unwrap(),
                auth_profile_ref: "production-app".into(),
                scale_set_name: "shaula-linux-x64".into(),
                runner_group: "Default".into(),
                labels: vec!["shaula-linux-x64".into()],
            },
            forgejo: None,
            capacity: CapacityPolicyDto {
                min_runners: 0,
                max_runners: 20,
            },
            template_profile_ref: TemplateProfileRefDto::from("kubernetes-linux-x64".to_string()),
            template_inputs: serde_json::Map::new(),
        }
    }

    #[test]
    fn valid_spec_passes() {
        assert!(validate_fleet_spec(&base_spec()).is_ok());
    }

    #[test]
    fn capacity_inversion_rejected() {
        let mut spec = base_spec();
        spec.capacity = CapacityPolicyDto {
            min_runners: 10,
            max_runners: 5,
        };
        let err = validate_fleet_spec(&spec).unwrap_err();
        assert_eq!(err.code, ReasonCode::SpecInvalid);
    }

    #[test]
    fn unknown_fields_rejected() {
        let raw = r#"{
            "github": {"target": {"kind":"organization","owner":"o"}, "auth_profile_ref":"a",
                       "scale_set_name":"s", "runner_group":"Default", "labels":[]},
            "capacity": {"min_runners":0,"max_runners":1},
            "template_profile_ref": "tpl",
            "template_inputs": {},
            "evil_field": {"namespace": "should-fail"}
        }"#;
        let parsed: Result<FleetSpec, _> = serde_json::from_str(raw);
        assert!(parsed.is_err(), "strict JSON must reject unknown fields");
    }

    #[test]
    fn bare_key_deserializes_and_legacy_pin_normalizes() {
        let bare: TemplateProfileRefDto = serde_json::from_str("\"tpl\"").unwrap();
        assert_eq!(bare.key(), "tpl");
        let legacy: TemplateProfileRefDto =
            serde_json::from_str(r#"{"key":"tpl","revision":4}"#).unwrap();
        assert_eq!(legacy.key(), "tpl");
        // The write form is always the bare key.
        assert_eq!(serde_json::to_string(&legacy).unwrap(), "\"tpl\"");
    }

    #[test]
    fn invalid_fleet_key_rejected() {
        assert!(FleetKey::new("has space").is_err());
        assert!(FleetKey::new("linux-x64.1_a").is_ok());
        // R8-01: no trim normalization — padded forms are rejected.
        assert!(FleetKey::new("\nlinux-x64").is_err());
        assert!(FleetKey::new("linux-x64 ").is_err());
    }
}

/// Parameter object for appending a fleet revision; keeps call sites
/// self-describing instead of positional.
#[derive(Debug, Clone)]
pub struct FleetRevisionInsert {
    pub key: String,
    pub incarnation: String,
    pub revision: i64,
    pub spec_json: String,
    pub template: Option<(String, i64, String, String)>,
    pub auth_desired: (String, i64),
    pub inputs_digest: String,
    pub actor: String,
    pub now: i64,
}
