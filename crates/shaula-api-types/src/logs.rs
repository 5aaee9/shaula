//! Sanitized diagnostic wire records; no raw commands or provider output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogQuery {
    pub cursor: Option<String>,
    pub phase: Option<String>,
    pub stream: Option<String>,
    pub limit_bytes: Option<usize>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    pub command_ordinal: u32,
    pub phase: String,
    pub stream: String,
    pub sequence: u64,
    pub text: String,
    pub observed_at: i64,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogPage {
    pub invocation_id: String,
    pub content_version: String,
    pub capture_status: String,
    pub entries: Vec<LogEntry>,
    pub next_cursor: Option<String>,
    pub has_gap: bool,
    pub lost_bytes: u64,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InvocationsPage {
    pub items: Vec<Invocation>,
    pub next_cursor: Option<String>,
    pub latest_create: Option<Invocation>,
    pub latest_destroy: Option<Invocation>,
}
