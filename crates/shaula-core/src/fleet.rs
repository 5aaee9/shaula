//! Fleet desired spec: canonical representation, normalization and
//! admission-time resolution facts (spec 0002 section 4).

use serde::{Deserialize, Serialize};

use crate::auth::validate_profile_key_for_fleet_ref;
use crate::auth::AuthRevisionRef;
use crate::capacity::CapacityPolicy;
use crate::error::{CoreError, CoreResult, ReasonCode};
use crate::github::{GitHubTarget, Label, ScaleSetIdentity};
use crate::template::TemplateProfileKey;

/// Client-submitted Fleet desired spec. Strict JSON rejects unknown fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSpec {
    pub github: FleetGithubSection,
    pub capacity: CapacityPolicyDto,
    pub template_profile_ref: TemplateProfileRefDto,
    /// Bounded template inputs; schema-checked against the pinned revision.
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
            Legacy { key: String, revision: u64 },
        }
        Ok(match Repr::deserialize(deserializer)? {
            Repr::Bare(key) | Repr::Legacy { key, .. } => Self(key),
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
    pub identity: ScaleSetIdentity,
    pub labels: Vec<Label>,
    pub capacity: CapacityPolicy,
    pub inputs_digest: String,
}

/// Canonical validation shared by create, replace and decommission paths.
pub fn validate_fleet_spec(spec: &FleetSpec) -> CoreResult<()> {
    if spec.github.scale_set_name.trim().is_empty() || spec.github.scale_set_name.len() > 100 {
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
    if spec.github.labels.len() > 32 {
        return Err(CoreError::new(ReasonCode::SpecInvalid, "at most 32 labels"));
    }
    for label in &spec.github.labels {
        let l = label.trim();
        if l.is_empty() || l.len() > 64 {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "each label must be 1..=64 characters",
            ));
        }
    }
    CapacityPolicy::from(spec.capacity)
        .validate()
        .map_err(|e| CoreError::new(ReasonCode::SpecInvalid, e))?;
    // The target kind has already been validated by typed deserialization.
    validate_profile_key_for_fleet_ref(&spec.github.auth_profile_ref)?;
    Ok(())
}

/// Normalizes a validated spec into its immutable identity parts.
pub fn normalize_fleet(spec: &FleetSpec, inputs_digest: String) -> CoreResult<NormalizedFleet> {
    let labels = spec
        .github
        .labels
        .iter()
        .map(|l| Label::system(l.trim()))
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(NormalizedFleet {
        identity: ScaleSetIdentity {
            target: spec.github.target.clone(),
            runner_group: spec.github.runner_group.trim().to_string(),
            scale_set_name: spec.github.scale_set_name.trim().to_string(),
        },
        labels,
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
            github: FleetGithubSection {
                target: GitHubTarget::organization("example-org").unwrap(),
                auth_profile_ref: "production-app".into(),
                scale_set_name: "shaula-linux-x64".into(),
                runner_group: "Default".into(),
                labels: vec!["shaula-linux-x64".into()],
            },
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
