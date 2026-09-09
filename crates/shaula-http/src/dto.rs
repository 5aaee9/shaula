//! Strict Serde DTOs. Unknown fields are rejected everywhere; secret
//! fields are write-only and appear in no response type.

use serde::{Deserialize, Serialize};

use shaula_core::fleet::{FleetGithubSection, TemplateProfileRefDto};

/// Fleet desired-state request body.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSpecDto {
    pub github: FleetGithubSection,
    pub capacity: shaula_core::fleet::CapacityPolicyDto,
    pub template_profile_ref: TemplateProfileRefDto,
    #[serde(default)]
    pub template_inputs: serde_json::Map<String, serde_json::Value>,
}

impl FleetSpecDto {
    pub fn into_domain(self) -> shaula_core::fleet::FleetSpec {
        shaula_core::fleet::FleetSpec {
            github: self.github,
            capacity: self.capacity,
            template_profile_ref: self.template_profile_ref,
            template_inputs: self.template_inputs,
        }
    }
}

/// Desired Fleet representation served by `GET /fleets/{key}`.
#[derive(Debug, Clone, Serialize)]
pub struct FleetResourceDto {
    pub key: String,
    pub spec: serde_json::Value,
    pub metadata: FleetMetadataDto,
    pub resolved: ResolvedDependenciesDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetMetadataDto {
    pub incarnation: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDependenciesDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<ResolvedTemplateDto>,
    pub auth_desired: AuthRefDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedTemplateDto {
    pub key: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub attestation_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthRefDto {
    pub profile_key: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetStatusDto {
    pub fleet_key: String,
    pub desired_revision: i64,
    pub observed_revision: i64,
    pub phase: String,
    pub dependencies: DependenciesDto,
    pub conditions: Vec<ConditionDto>,
    pub capacity: CapacityDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependenciesDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_profile: Option<TemplateDependencyDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_auth: Option<AuthDependencyDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateDependencyDto {
    pub key: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub attestation_id: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthDependencyDto {
    pub desired: AuthRefDto,
    pub observed: Option<AuthRefDto>,
    pub handoff_state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConditionDto {
    #[serde(rename = "type")]
    pub condition_type: String,
    pub status: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapacityDto {
    pub assigned_demand: i64,
    pub target: i64,
    pub effective: i64,
    pub occupancy: i64,
}

/// Accepted mutation response (`202`).
#[derive(Debug, Clone, Serialize)]
pub struct MutationAcceptedDto {
    pub change_id: String,
    pub change_state: String,
    pub revision: i64,
    pub etag: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeDto {
    pub id: String,
    pub resource_kind: String,
    pub resource_key: String,
    pub revision: i64,
    pub kind: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Template Profile request body; `platform`/`bindings_contract` are
/// deliberately absent — strict deserialization rejects them as a second
/// authority.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateProfilePutDto {
    pub artifact_digest: String,
    pub engine_ref: String,
    #[serde(default)]
    pub bindings: serde_json::Value,
    #[serde(default)]
    pub fleet_input_policy: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateProfileViewDto {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_revision: Option<i64>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bindings_contract: Option<String>,
    pub bindings_present: bool,
}

/// Strict GitHub App policy request. Unknown legacy fields are rejected,
/// including null or empty values; credentials remain write-only.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthProfilePutDto {
    pub kind: String,
    #[serde(default)]
    pub schema_version: Option<i64>,
    #[serde(default)]
    pub app_id: Option<String>,
    /// Write-only GitHub App private key bytes.
    #[serde(default)]
    pub private_key: Option<String>,
    /// v2 Target policy selectors (schema_version 2).
    #[serde(default)]
    pub target_policy: Option<Vec<shaula_core::auth_policy::TargetSelector>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationPutDto {
    pub result: String,
    /// The TYPED canonical subject (R7-04): its own `deny_unknown_fields`
    /// makes submissions carrying excluded payload (raw test output,
    /// bindings, credentials) or missing canonical members a 422 at this
    /// strict-parse boundary — never a durable record.
    pub subject: shaula_core::registry::AttestationSubject,
    #[serde(default)]
    pub evidence_digest: Option<String>,
    pub suite: SuiteDto,
    pub completed_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteDto {
    pub name: String,
    pub version: String,
}
