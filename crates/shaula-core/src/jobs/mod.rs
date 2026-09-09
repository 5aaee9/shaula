//! Workflow-job observations and read-only history. None of these types authorize lifecycle effects.

mod metadata;
mod projection;

pub use metadata::JobMetadata;
pub use projection::{project_observations, JobProjection};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedStatus {
    Queued,
    Assigned,
    Running,
    Completed,
    AssignmentWithdrawn,
    Unknown,
}

impl ObservedStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Assigned => "assigned",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::AssignmentWithdrawn => "assignment_withdrawn",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociationStatus {
    Unverified,
    Verified,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Available,
    Assigned,
    Started,
    Completed,
}

impl ObservationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Assigned => "Assigned",
            Self::Started => "Started",
            Self::Completed => "Completed",
        }
    }
}

/// An approved observation, never the raw Actions Service message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobObservation {
    pub id: String,
    pub kind: ObservationKind,
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
    pub association_status: AssociationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: String,
    pub fleet_key: String,
    pub fleet_incarnation: String,
    pub scale_set_id: i64,
    pub protocol_job_id: String,
    #[serde(flatten)]
    pub metadata: JobMetadata,
    pub observed_status: ObservedStatus,
    pub reported_result: Option<String>,
    /// `unknown` is used when no independent listener-health proof is available.
    pub freshness: String,
    pub association_status: AssociationStatus,
    pub github_run_url: Option<String>,
    pub actions_job_id: Option<i64>,
    pub workflow_run_attempt: Option<i64>,
    pub github_conclusion: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDetail {
    #[serde(flatten)]
    pub job: JobSummary,
    pub observations: Vec<JobObservation>,
    pub observations_truncated: bool,
    pub generations: Vec<GenerationSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationSummary {
    pub id: String,
    pub fleet_key: String,
    pub fleet_incarnation: Option<String>,
    pub runner_name: String,
    pub generation_name: String,
    pub github_runner_id: Option<i64>,
    pub state: String,
    pub subphase: Option<String>,
    pub template_profile_key: String,
    pub template_revision: i64,
    pub association_status: AssociationStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationDetail {
    #[serde(flatten)]
    pub generation: GenerationSummary,
    pub jobs: Vec<JobSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobsPage {
    pub items: Vec<JobSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationsPage {
    pub items: Vec<GenerationSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobsReadError {
    #[error("invalid jobs query: {0}")]
    InvalidQuery(&'static str),
    #[error("job history is temporarily unavailable")]
    Unavailable,
}

/// History reads are separate from lifecycle/reconciliation permissions.
#[async_trait::async_trait]
pub trait JobsReadPort: Send + Sync {
    async fn list_jobs(&self, query: JobsQuery) -> Result<JobsPage, JobsReadError>;
    async fn get_job(&self, id: &str) -> Result<Option<JobDetail>, JobsReadError>;
    async fn list_generations(
        &self,
        query: GenerationsQuery,
    ) -> Result<GenerationsPage, JobsReadError>;
    async fn get_generation(&self, id: &str) -> Result<Option<GenerationDetail>, JobsReadError>;
}

#[cfg(test)]
mod tests;
