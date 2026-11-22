//! Aggregate ledger reads for the metrics scrape: counts grouped by fleet
//! and fixed status codes only. Rows, IDs and payloads never cross here.

use async_trait::async_trait;

use crate::error::CoreResult;

/// Ledger rows with one fleet and state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationCount {
    pub fleet_key: String,
    pub state: String,
    pub count: i64,
}

/// Retained job records with one fleet, backend, status and result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobCount {
    pub fleet_key: String,
    pub backend: &'static str,
    pub status: String,
    pub result: Option<String>,
    pub count: i64,
}

/// Lifecycle worker ledger rows grouped by phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerPhaseCount {
    pub phase: String,
    pub count: i64,
}

/// Configured lifecycle worker admission limits exposed at scrape time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerAdmissionLimits {
    pub max_workers: usize,
    pub recovery_reserve: usize,
    pub create_limit: usize,
}

#[async_trait]
pub trait LedgerMetricsPort: Send + Sync {
    async fn generation_counts(&self) -> CoreResult<Vec<GenerationCount>>;
    async fn job_counts(&self) -> CoreResult<Vec<JobCount>>;
    async fn worker_phase_counts(&self) -> CoreResult<Vec<WorkerPhaseCount>>;
}
