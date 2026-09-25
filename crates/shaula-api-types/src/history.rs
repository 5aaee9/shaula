//! Public history projections. Unknown state strings remain observable.
use serde_json::Value;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobMetadata {
    pub owner_name: Option<String>,
    pub repository_name: Option<String>,
    pub job_display_name: Option<String>,
    pub job_workflow_ref: Option<String>,
    pub workflow_run_id: Option<i64>,
    pub event_name: Option<String>,
    pub queue_time: Option<String>,
    pub scale_set_assign_time: Option<String>,
    pub runner_assign_time: Option<String>,
    pub finish_time: Option<String>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobObservation {
    pub id: String,
    pub kind: String,
    /// The wire value: zero means an unknown request on a direct assignment.
    /// Only positive values can join observations by request identity.
    pub runner_request_id: i64,
    pub runner_id: Option<i64>,
    pub runner_name: Option<String>,
    pub metadata: JobMetadata,
    pub reported_result: Option<String>,
    pub epoch: i64,
    pub message_id: i64,
    pub observed_at: i64,
    pub generation_id: Option<String>,
    pub association_status: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobSummary {
    pub id: String,
    pub fleet_key: String,
    pub fleet_incarnation: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forgejo: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_set_id: Option<i64>,
    pub protocol_job_id: String,
    #[serde(flatten)]
    pub metadata: JobMetadata,
    pub observed_status: String,
    pub reported_result: Option<String>,
    /// `unknown` is used when no independent listener-health proof is available.
    pub freshness: String,
    pub association_status: String,
    pub github_run_url: Option<String>,
    pub actions_job_id: Option<i64>,
    pub workflow_run_attempt: Option<i64>,
    pub github_conclusion: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobDetail {
    #[serde(flatten)]
    pub job: JobSummary,
    pub observations: Vec<JobObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forgejo_observations: Vec<Value>,
    pub observations_truncated: bool,
    pub generations: Vec<GenerationSummary>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GenerationSummary {
    pub id: String,
    pub fleet_key: String,
    pub fleet_incarnation: Option<String>,
    pub runner_name: String,
    pub generation_name: String,
    pub github_runner_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forgejo_runner_id: Option<String>,
    pub state: String,
    pub subphase: Option<String>,
    pub template_profile_key: String,
    pub template_revision: i64,
    pub association_status: String,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GenerationDetail {
    #[serde(flatten)]
    pub generation: GenerationSummary,
    pub jobs: Vec<JobSummary>,
}
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobsQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub fleet_key: Option<String>,
    pub status: Option<String>,
    pub repository: Option<String>,
    pub job_name: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationsQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub fleet_key: Option<String>,
    pub status: Option<String>,
    pub association: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobsPage {
    pub items: Vec<JobSummary>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GenerationsPage {
    pub items: Vec<GenerationSummary>,
    pub next_cursor: Option<String>,
}
