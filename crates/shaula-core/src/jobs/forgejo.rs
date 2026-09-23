//! Forgejo observations are snapshots, not GitHub assignments or completion proofs.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgejoJobState {
    pub target: crate::forgejo::ForgejoTarget,
    // Decimal strings preserve u64 IDs in browser clients.
    pub repository_id: String,
    pub job_id: String,
    pub attempt: String,
    pub run_id: String,
    pub task_id: String,
    pub runs_on: Vec<String>,
    pub last_reported_status: String,
    pub last_observed_at: i64,
    pub in_snapshot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<super::ForgejoJobResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment_attempted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgejoJobObservation {
    pub id: String,
    /// None means absent from a successful complete snapshot, never completed.
    pub reported_status: Option<String>,
    pub task_id: String,
    pub observed_at: i64,
    /// Missing on older stored events, which all came from runner snapshots.
    #[serde(default)]
    pub source: ForgejoObservationSource,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgejoObservationSource {
    #[default]
    RunnerSnapshot,
    TaskHistory,
}

/// Captured Fleet authority fences both successful snapshots and failures.
/// None marks the previous snapshot stale without refreshing its timestamp.
#[async_trait::async_trait]
pub trait ForgejoJobsStore: Send + Sync {
    async fn forgejo_jobs_snapshot(
        &self,
        fleet: &str,
        guard: &crate::registry::FleetRuntimeGuard,
        jobs: Option<&[crate::ports::forgejo::ForgejoJob]>,
        now: i64,
    ) -> crate::error::CoreResult<bool>;

    /// Reserves a bounded, rate-limited read batch; never authorizes resource effects.
    async fn forgejo_jobs_pending_results(
        &self,
        fleet: &str,
        guard: &crate::registry::FleetRuntimeGuard,
        now: i64,
    ) -> crate::error::CoreResult<Vec<super::ForgejoJobLookup>>;

    async fn forgejo_job_result(
        &self,
        fleet: &str,
        guard: &crate::registry::FleetRuntimeGuard,
        lookup: &super::ForgejoJobLookup,
        result: &super::ForgejoTaskResult,
        now: i64,
    ) -> crate::error::CoreResult<bool>;
}
