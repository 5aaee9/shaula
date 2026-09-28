//! Non-secret discovery of stored template sources and declared Terraform inputs.

use serde::{Deserialize, Serialize};

use super::InputOption;

/// Rebuilds and verifies executable material against the immutable database archive.
#[async_trait::async_trait]
pub trait ArtifactCache: Send + Sync {
    async fn ensure_cached(&self, digest: &str) -> crate::error::CoreResult<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSource {
    pub key: String,
    pub artifact_digest: String,
    pub platform: String,
    pub engine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateVariable {
    pub key: String,
    pub label: String,
    pub description: String,
    pub type_name: String,
    pub required: bool,
    pub sensitive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_json: Option<String>,
    pub options: Vec<InputOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateVariables {
    pub artifact_digest: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub bindings: Vec<TemplateVariable>,
    pub parameters: Vec<TemplateVariable>,
}

/// Authorized non-secret metadata of one stored archive; never its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateArtifactMetadata {
    pub digest: String,
    pub size_bytes: i64,
    pub created_at: i64,
    /// Default catalog sources currently naming this archive.
    pub source_keys: Vec<String>,
    /// Template Revisions pinning this archive, oldest first, bounded.
    pub revisions: Vec<TemplateArtifactReference>,
    pub revisions_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateArtifactReference {
    pub profile_key: String,
    pub revision: i64,
    pub state: String,
}
