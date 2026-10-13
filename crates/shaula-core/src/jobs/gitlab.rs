//! GitLab job projection (spec 0043 §6.1). A record exists only for a job a
//! Generation's own registration ran, so its association is `Verified`; later
//! exact reads refresh the status but never add, move or clear that association.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabJobState {
    pub target: crate::gitlab::GitlabTarget,
    // Decimal strings preserve u64 IDs in browser clients.
    pub project_id: String,
    pub project_path: String,
    pub job_id: String,
    pub pipeline_id: String,
    pub job_name: String,
    /// Constructed beneath the configured instance, never response-supplied.
    pub web_url: String,
    pub last_reported_status: String,
    pub last_observed_at: i64,
    /// False once an exact read in the proven scope no longer finds the job.
    pub listed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<GitlabJobResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment_attempted_at: Option<i64>,
}

/// The job's own terminal status, read from the exact job resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabJobResult {
    pub status: GitlabJobConclusion,
    pub observed_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitlabJobConclusion {
    Success,
    Failed,
    Canceled,
    Skipped,
}

impl GitlabJobConclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(status: &str) -> Option<Self> {
        match status {
            "success" => Some(Self::Success),
            "failed" => Some(Self::Failed),
            "canceled" => Some(Self::Canceled),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// `pending` → queued, `running` → running; every other non-terminal status
/// (created, preparing, manual, canceling, …) stays unknown.
pub fn gitlab_observed_status(status: &str) -> super::ObservedStatus {
    match status {
        "pending" => super::ObservedStatus::Queued,
        "running" => super::ObservedStatus::Running,
        _ if GitlabJobConclusion::parse(status).is_some() => super::ObservedStatus::Completed,
        _ => super::ObservedStatus::Unknown,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabJobObservation {
    pub id: String,
    /// None means the exact read no longer found the job, never completion.
    pub reported_status: Option<String>,
    pub source: GitlabObservationSource,
    /// The registration that reported the job; absent on job reads after the
    /// registration was deleted (GitLab then reports `runner: null`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    pub observed_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitlabObservationSource {
    /// The Generation registration's own running-jobs list (the claim).
    RunnerJobs,
    /// An exact `project_id/job_id` read by the enrichment worker.
    JobRead,
}

/// Captured local job identity. The store rechecks its scope at commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabJobLookup {
    pub record_id: String,
    pub project_id: u64,
    pub job_id: u64,
}

/// Captured Fleet authority fences every write; none authorizes lifecycle effects.
#[async_trait::async_trait]
pub trait GitlabJobsStore: Send + Sync {
    /// Records a job listed as running on a Generation's registration. Only a
    /// job matching the Generation's persisted claim creates or links a record.
    async fn gitlab_job_running(
        &self,
        fleet: &str,
        guard: &crate::registry::FleetRuntimeGuard,
        generation_id: &str,
        runner_id: u64,
        job: &crate::ports::gitlab::GitlabRunningJob,
        now: i64,
    ) -> crate::error::CoreResult<bool>;

    /// Reserves a bounded, rate-limited read batch of non-terminal records.
    async fn gitlab_jobs_pending(
        &self,
        fleet: &str,
        guard: &crate::registry::FleetRuntimeGuard,
        now: i64,
    ) -> crate::error::CoreResult<Vec<GitlabJobLookup>>;

    /// Applies one exact read; `None` marks the job as no longer listed.
    async fn gitlab_job_read(
        &self,
        fleet: &str,
        guard: &crate::registry::FleetRuntimeGuard,
        lookup: &GitlabJobLookup,
        read: Option<&crate::ports::gitlab::GitlabJobDetail>,
        now: i64,
    ) -> crate::error::CoreResult<bool>;
}
