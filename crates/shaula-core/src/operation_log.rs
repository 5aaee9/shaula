//! Sanitized operation diagnostics. These records never authorize resource effects.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::CoreResult;

#[path = "operation_log_page.rs"]
mod page;
pub use page::{InvocationQuery, InvocationsPage};

pub const SANITIZATION_POLICY: &str = "shaula.operation-text/v2";

/// Earlier sanitized archives remain readable; an unknown policy fails closed.
pub fn supported_sanitization_policy(policy: &str) -> bool {
    matches!(policy, "shaula.operation-text/v1" | SANITIZATION_POLICY)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogConfig {
    pub invocation_bytes: u64,
    pub quota_bytes: u64,
    pub retention_days: u32,
    pub metadata_retention_days: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            invocation_bytes: 32 * 1024 * 1024,
            quota_bytes: 10 * 1024 * 1024 * 1024,
            retention_days: 30,
            metadata_retention_days: 90,
        }
    }
}

impl LogConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(128 * 1024..=256 * 1024 * 1024).contains(&self.invocation_bytes)
            || self.quota_bytes < self.invocation_bytes
            || self.quota_bytes > i64::MAX as u64
            || !(1..=365).contains(&self.retention_days)
            || !(self.retention_days..=3650).contains(&self.metadata_retention_days)
        {
            return Err("operation log limits are outside supported bounds".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BeginInvocation {
    pub generation_id: String,
    pub operation: String,
    pub started_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invocation {
    pub id: String,
    pub generation_id: String,
    pub fleet_key: String,
    pub fleet_incarnation: Option<String>,
    pub template_profile_key: String,
    pub template_revision: i64,
    pub template_artifact_digest: String,
    pub operation: String,
    pub ordinal: i64,
    pub effect_attempt_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub capture_sealed_at: Option<i64>,
    pub execution_outcome: String,
    pub capture_status: String,
    pub reason: Option<String>,
    pub content_version: String,
    pub policy_version: String,
    pub retained_bytes: u64,
    pub lost_bytes: u64,
    pub commands: Vec<LogCommand>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogCommand {
    pub invocation_id: String,
    pub ordinal: u32,
    pub phase: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub termination: String,
    pub effect_attempt_id: Option<String>,
    pub capture_partial: bool,
    pub lost_bytes: u64,
}

/// Text passed here has already crossed the Runtime's sanitization boundary.
#[derive(Clone, Serialize, Deserialize)]
pub struct AppendLog {
    pub invocation_id: String,
    pub command_ordinal: u32,
    pub phase: String,
    pub stream: String,
    pub sequence: u64,
    pub text: String,
    pub observed_at: i64,
    pub withheld: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinishInvocation {
    pub invocation_id: String,
    pub execution_outcome: String,
    pub ended_at: i64,
    pub lost_bytes: u64,
    pub partial: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogQuery {
    pub cursor: Option<String>,
    pub phase: Option<String>,
    pub stream: Option<String>,
    pub limit_bytes: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub command_ordinal: u32,
    pub phase: String,
    pub stream: String,
    pub sequence: u64,
    pub text: String,
    pub observed_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogPage {
    pub invocation_id: String,
    pub content_version: String,
    pub capture_status: String,
    pub entries: Vec<LogEntry>,
    pub next_cursor: Option<String>,
    pub has_gap: bool,
    pub lost_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetupProjection {
    /// pending, ready, withheld, unavailable, or not_recorded.
    pub status: String,
    pub invocation_id: Option<String>,
    pub content_version: Option<String>,
    pub detail: Option<String>,
    pub partial: bool,
}

#[async_trait]
pub trait OperationLogReadPort: Send + Sync {
    async fn list_invocations(&self, generation_id: &str) -> CoreResult<Vec<Invocation>>;
    async fn list_invocations_page(
        &self,
        generation_id: &str,
        query: InvocationQuery,
    ) -> CoreResult<InvocationsPage> {
        query.page(generation_id, self.list_invocations(generation_id).await?)
    }
    async fn read_page(&self, invocation_id: &str, query: LogQuery) -> CoreResult<LogPage>;
    async fn setup_projection(&self, generation_id: &str) -> CoreResult<SetupProjection>;
}

#[async_trait]
pub trait OperationLogSink: Send + Sync {
    async fn begin(&self, request: BeginInvocation) -> CoreResult<String>;
    async fn append(&self, chunk: AppendLog) -> CoreResult<()>;
    async fn command(&self, command: LogCommand) -> CoreResult<()>;
    async fn finish(&self, result: FinishInvocation) -> CoreResult<()>;
}
