//! Strict request envelopes retaining nested JSON verbatim, including null/omission.
use crate::Document;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetPut {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forgejo: Option<Document>,
    pub capacity: Document,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_profile_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_pool: Option<PoolPut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_pool_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_inputs: Option<Document>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolPut {
    pub members: Vec<PoolMember>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_policy: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolMember {
    pub key: String,
    pub template_profile_ref: String,
    pub weight: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_inputs: Option<Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runners: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplatePublish {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_key: Option<String>,
    pub artifact_digest: String,
    pub engine_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_input_policy: Option<Document>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_key: Option<String>,
    pub artifact_digest: String,
    pub engine_ref: String,
    #[serde(
        default,
        deserialize_with = "object",
        skip_serializing_if = "Option::is_none"
    )]
    pub bindings: Option<Document>,
    #[serde(
        default,
        deserialize_with = "object",
        skip_serializing_if = "Option::is_none"
    )]
    pub fleet_input_policy: Option<Document>,
}
fn object<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Document>, D::Error> {
    let value = Document::deserialize(deserializer)?;
    if !value.raw().trim_start().starts_with('{') {
        return Err(serde::de::Error::custom(
            "an object is required; omit to inherit",
        ));
    }
    Ok(Some(value))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyUpdate {
    pub base_revision: i64,
    pub target_policy: Document,
}

/// Opaque secret-bearing body. Debug is redacted and public serialization is explicit.
#[derive(Debug, Clone)]
pub struct AuthPublish(Document);
impl AuthPublish {
    pub fn parse(raw: String) -> Result<Self, serde_json::Error> {
        let value = Document::parse(raw)?;
        #[derive(Deserialize)]
        #[serde(tag = "kind", deny_unknown_fields)]
        enum Wire {
            #[serde(rename = "github_app")]
            Github {
                schema_version: i64,
                app_id: String,
                private_key: String,
                target_policy: Document,
            },
            #[serde(rename = "forgejo_token")]
            Forgejo {
                instance_url: String,
                scope: Document,
                token: String,
            },
        }
        // Destructure to ensure fields are deliberately checked rather than silently dropped.
        let valid = match value.decode::<Wire>()? {
            Wire::Github {
                schema_version,
                app_id,
                private_key,
                target_policy,
            } => {
                schema_version == 2
                    && !app_id.is_empty()
                    && !private_key.is_empty()
                    && target_policy.raw().starts_with('[')
            }
            Wire::Forgejo {
                instance_url,
                scope,
                token,
            } => !instance_url.is_empty() && !token.is_empty() && scope.raw().starts_with('{'),
        };
        if !valid {
            return Err(<serde_json::Error as serde::de::Error>::custom(
                "invalid authentication publication",
            ));
        }
        Ok(Self(value))
    }
    pub fn document(&self) -> &Document {
        &self.0
    }
}
