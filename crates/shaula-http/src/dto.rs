//! Strict Serde DTOs. Unknown fields are rejected everywhere; secret
//! fields are write-only and appear in no response type.

use serde::{Deserialize, Deserializer, Serialize};

use shaula_core::fleet::{
    FleetForgejoSection, FleetGithubSection, FleetProviderKind, TemplateProfileRefDto,
};

/// Fleet desired-state request body.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSpecDto {
    #[serde(default)]
    pub kind: FleetProviderKind,
    #[serde(default)]
    pub github: Option<FleetGithubSection>,
    #[serde(default)]
    pub forgejo: Option<FleetForgejoSection>,
    pub capacity: shaula_core::fleet::CapacityPolicyDto,
    pub template_profile_ref: TemplateProfileRefDto,
    #[serde(default)]
    pub template_inputs: serde_json::Map<String, serde_json::Value>,
}

impl FleetSpecDto {
    pub fn into_domain(self) -> shaula_core::fleet::FleetSpec {
        shaula_core::fleet::FleetSpec {
            kind: self.kind,
            github: self.github.unwrap_or_default(),
            forgejo: self.forgejo,
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
    #[serde(default)]
    pub source_key: Option<String>,
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

/// Strict provider-specific auth request. Unknown legacy fields are
/// rejected, including null or empty values; credentials remain write-only.
#[derive(Debug, Clone)]
pub struct AuthProfilePutDto {
    pub kind: String,
    pub schema_version: Option<i64>,
    pub app_id: Option<String>,
    /// Write-only GitHub App private key bytes.
    pub private_key: Option<String>,
    /// Write-only Forgejo token bytes.
    pub token: Option<String>,
    /// Forgejo target authority, required for `forgejo_token`.
    pub instance_url: Option<String>,
    pub scope: Option<shaula_core::forgejo::ForgejoScope>,
    /// v2 Target policy selectors (schema_version 2).
    pub target_policy: Option<Vec<shaula_core::auth_policy::TargetSelector>>,
}

impl<'de> Deserialize<'de> for AuthProfilePutDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: String,
            #[serde(default)]
            schema_version: Option<i64>,
            #[serde(default)]
            app_id: Option<String>,
            #[serde(default)]
            private_key: Option<String>,
            #[serde(default)]
            token: Option<String>,
            #[serde(default)]
            instance_url: Option<String>,
            #[serde(default)]
            scope: Option<shaula_core::forgejo::ForgejoScope>,
            #[serde(default)]
            target_policy: Option<Vec<shaula_core::auth_policy::TargetSelector>>,
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        let wire: Wire = serde_json::from_value(value.clone()).map_err(serde::de::Error::custom)?;
        let object = value.as_object();
        let forgejo_fields_present = object.is_some_and(|object| {
            object.contains_key("token")
                || object.contains_key("instance_url")
                || object.contains_key("scope")
        });
        let token = wire.token;
        let instance_url = wire.instance_url;
        let scope = wire.scope;
        if wire.kind != "forgejo_token" && forgejo_fields_present {
            return Err(serde::de::Error::custom(
                "Forgejo authentication fields require kind: forgejo_token",
            ));
        }
        Ok(Self {
            kind: wire.kind,
            schema_version: wire.schema_version,
            app_id: wire.app_id,
            private_key: wire.private_key,
            token,
            instance_url,
            scope,
            target_policy: wire.target_policy,
        })
    }
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
