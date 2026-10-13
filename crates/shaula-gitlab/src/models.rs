//! Raw GitLab REST payloads. Only fields the adapter relies on are decoded.

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(crate) struct RawGroup {
    pub id: u64,
    pub full_path: String,
}

#[derive(Deserialize)]
pub(crate) struct RawProject {
    pub id: u64,
    pub path_with_namespace: String,
}

#[derive(Deserialize)]
pub(crate) struct RawRegistration {
    pub id: u64,
    pub token: String,
}

#[derive(Serialize)]
pub(crate) struct RegisterRunnerRequest<'a> {
    pub runner_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<u64>,
    pub description: &'a str,
    pub maintenance_note: &'a str,
    pub tag_list: &'a [String],
    pub run_untagged: bool,
    pub locked: bool,
    pub paused: bool,
    pub access_level: &'static str,
}

#[derive(Deserialize)]
pub(crate) struct RawRunnerSummary {
    pub id: u64,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawScopeRef {
    pub id: u64,
}

#[derive(Deserialize)]
pub(crate) struct RawRunner {
    pub id: u64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub maintenance_note: Option<String>,
    pub status: String,
    #[serde(default)]
    pub job_execution_status: Option<String>,
    pub paused: bool,
    #[serde(default)]
    pub tag_list: Vec<String>,
    pub run_untagged: bool,
    pub runner_type: String,
    #[serde(default)]
    pub groups: Vec<RawScopeRef>,
    #[serde(default)]
    pub projects: Vec<RawScopeRef>,
}

#[derive(Deserialize)]
pub(crate) struct RawJobPipeline {
    pub id: u64,
}

#[derive(Deserialize)]
pub(crate) struct RawJobProject {
    pub id: u64,
    pub path_with_namespace: String,
}

#[derive(Deserialize)]
pub(crate) struct RawJob {
    pub id: u64,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub web_url: Option<String>,
    pub pipeline: RawJobPipeline,
    pub project: RawJobProject,
}

#[derive(Deserialize)]
pub(crate) struct RawVersion {
    pub version: String,
}

#[derive(Deserialize)]
pub(crate) struct RawTokenSelf {
    pub active: bool,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawJobDetailPipeline {
    pub id: u64,
    pub project_id: u64,
}

/// `GET /projects/:id/jobs/:job_id`. `runner` is null once the registration
/// that ran the job has been deleted; `web_url` is deliberately not decoded.
#[derive(Deserialize)]
pub(crate) struct RawJobDetail {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub pipeline: RawJobDetailPipeline,
    #[serde(default)]
    pub runner: Option<RawScopeRef>,
}
