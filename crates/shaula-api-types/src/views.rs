//! Registry projections. TypedResource also retains the complete original response.
use crate::{Document, RevisionMetadata};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSummary {
    pub key: String,
    pub revision: i64,
    pub incarnation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetsList {
    pub fleets: Vec<ResourceSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolsList {
    pub pools: Vec<ResourceSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilesList {
    pub profiles: Vec<Profile>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    pub active_revision: Option<i64>,
    pub status: String,
    pub runner_backend: Option<String>,
    pub kind: Option<String>,
    #[serde(rename = "credential_present")]
    pub credential_present: Option<bool>,
    #[serde(rename = "schema_version")]
    pub schema_version: Option<i64>,
    pub active: Option<Document>,
    pub desired: Option<Document>,
    pub live_fleets: Option<Vec<Document>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    pub key: String,
    pub spec: Document,
    pub metadata: RevisionMetadata,
    pub resolved: Document,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetStatus {
    pub fleet_key: String,
    pub desired_revision: i64,
    pub observed_revision: i64,
    pub phase: String,
    pub github_auth: Option<Document>,
    pub conditions: Vec<Document>,
    pub capacity: Document,
    pub last_error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStatus {
    pub key: String,
    pub desired_revision: i64,
    pub active_revision: Option<i64>,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileImpact {
    pub desired_revision: i64,
    pub live_fleets: Vec<Document>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateRevision {
    pub profile_key: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub engine_ref: String,
    pub source_key: Option<String>,
    pub platform: Option<String>,
    pub bindings_contract: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub bindings_present: bool,
    pub bindings: Option<Document>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthRevision {
    pub profile_key: String,
    pub revision: i64,
    pub kind: String,
    pub app_id: Option<String>,
    #[serde(rename = "schema_version")]
    pub schema_version: i64,
    pub state: String,
    pub reason: Option<String>,
    #[serde(rename = "target_policy")]
    pub target_policy: Option<Document>,
    pub bindings: Option<Document>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSource {
    pub key: String,
    pub artifact_digest: String,
    pub platform: String,
    pub engine_ref: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateSources {
    pub sources: Vec<TemplateSource>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputOption {
    pub value_json: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateVariable {
    pub key: String,
    pub label: String,
    pub description: String,
    pub type_name: String,
    pub required: bool,
    pub sensitive: bool,
    pub default_value_json: Option<String>,
    pub options: Vec<InputOption>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateVariables {
    pub artifact_digest: String,
    pub available: bool,
    pub reason: Option<String>,
    pub bindings: Vec<TemplateVariable>,
    pub parameters: Vec<TemplateVariable>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputField {
    pub key: String,
    pub label: String,
    pub description: String,
    pub required: bool,
    pub options: Vec<InputOption>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputContract {
    pub version: u8,
    pub profile_key: String,
    pub incarnation: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub mode: String,
    pub fields: Option<Vec<InputField>>,
    pub presets: Option<Vec<InputOption>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attestation {
    pub profile_key: String,
    pub revision: i64,
    pub subject: Document,
    pub result: String,
    pub suite: Document,
    pub completed_at: i64,
    pub subject_verified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthInstallationLink {
    pub url: String,
    pub app_id: String,
    pub revision: i64,
    pub incarnation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactUpload {
    pub digest: String,
    pub size_bytes: u64,
}
