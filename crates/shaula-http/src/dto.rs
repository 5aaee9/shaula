//! Strict Serde DTOs. Unknown fields are rejected everywhere; secret
//! fields are write-only and appear in no response type.

use serde::{Deserialize, Deserializer};

use shaula_core::fleet::{
    FleetForgejoSection, FleetGithubSection, FleetProviderKind, TemplatePoolSpec,
    TemplateProfileRefDto,
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
    #[serde(default)]
    pub template_profile_ref: TemplateProfileRefDto,
    #[serde(default)]
    pub template_pool: Option<TemplatePoolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_pool_ref: Option<String>,
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
            template_pool: self.template_pool,
            template_pool_ref: self.template_pool_ref,
            template_inputs: self.template_inputs,
        }
    }
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
