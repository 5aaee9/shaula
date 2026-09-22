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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgejoJobObservation {
    pub id: String,
    /// None means absent from a successful complete snapshot, never completed.
    pub reported_status: Option<String>,
    pub task_id: String,
    pub observed_at: i64,
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
}
