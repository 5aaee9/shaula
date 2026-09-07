//! Strict Serde wire DTOs mirroring the pinned `actions/scaleset` protocol.
//! These types never cross a core port; `shaula-core` owns domain models.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SCALE_SET_ENDPOINT: &str = "_apis/runtime/runnerscalesets";
pub const RUNNER_ENDPOINT: &str = "_apis/distributedtask/pools/0/agents";

pub const API_VERSION: &str = "6.0-preview";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    #[serde(rename = "type")]
    pub label_type: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunnerSetting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_update: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunnerScaleSetStatistic {
    #[serde(default)]
    pub total_available_jobs: i64,
    #[serde(default)]
    pub total_acquired_jobs: i64,
    #[serde(default)]
    pub total_assigned_jobs: i64,
    #[serde(default)]
    pub total_running_jobs: i64,
    #[serde(default)]
    pub total_registered_runners: i64,
    #[serde(default)]
    pub total_busy_runners: i64,
    #[serde(default)]
    pub total_idle_runners: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerScaleSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_group_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_group_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub runner_setting: RunnerSetting,
    #[serde(default)]
    pub created_on: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statistics: Option<RunnerScaleSetStatistic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerScaleSetList {
    pub count: i64,
    #[serde(default)]
    pub value: Vec<RunnerScaleSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerGroup {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default, rename = "isDefaultGroup")]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerGroupList {
    pub count: i64,
    #[serde(default)]
    pub value: Vec<RunnerGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionBody {
    pub owner_name: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerScaleSetSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_queue_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_queue_access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statistics: Option<RunnerScaleSetStatistic>,
}

// Manual Debug (F11): the queue access token is a bearer credential and
// must never reach a trace through a derived Debug — even inside a wire
// struct. Serde wire behavior is unaffected.
impl std::fmt::Debug for RunnerScaleSetSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerScaleSetSession")
            .field("session_id", &self.session_id)
            .field("owner_name", &self.owner_name)
            .field("message_queue_url", &self.message_queue_url)
            .field("message_queue_access_token", &"REDACTED")
            .field("statistics", &self.statistics)
            .finish()
    }
}

/// Raw batched message: `body` is a JSON-encoded array of typed messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerScaleSetMessageResponse {
    #[serde(rename = "messageId")]
    pub message_id: i64,
    #[serde(rename = "messageType")]
    pub message_type: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub statistics: Option<RunnerScaleSetStatistic>,
}

pub const MESSAGE_TYPE_JOB_MESSAGES: &str = "RunnerScaleSetJobMessages";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquireJobsResponse {
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub value: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerReference {
    pub id: i64,
    pub name: String,
    #[serde(rename = "runnerScaleSetId", default)]
    pub runner_scale_set_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerReferenceList {
    pub count: i64,
    #[serde(default)]
    pub value: Vec<RunnerReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerScaleSetJitRunnerSetting {
    pub name: String,
    #[serde(rename = "workFolder")]
    pub work_folder: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerScaleSetJitRunnerConfig {
    #[serde(default)]
    pub runner: Option<RunnerReference>,
    #[serde(rename = "encodedJITConfig")]
    pub encoded_jit_config: String,
}

/// Actions service exception body used for typed error classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsException {
    #[serde(rename = "typeName", default)]
    pub type_name: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RegistrationToken {
    pub token: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ActionsServiceAdminConnection {
    #[serde(rename = "url")]
    pub actions_service_url: String,
    #[serde(rename = "token")]
    pub admin_token: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstallationAccessToken {
    pub token: String,
    #[serde(rename = "expires_at")]
    pub expires_at: String,
}

// Type-bound redaction (spec 0007 §3.2): every wire type carrying a
// credential hides the secret from Debug while keeping protocol
// serialization intact.
impl std::fmt::Debug for RunnerScaleSetJitRunnerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerScaleSetJitRunnerConfig")
            .field("runner", &self.runner)
            .field("encoded_jit_config", &"[REDACTED]")
            .finish()
    }
}

impl std::fmt::Debug for RegistrationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrationToken")
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl std::fmt::Debug for ActionsServiceAdminConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionsServiceAdminConnection")
            .field("actions_service_url", &self.actions_service_url)
            .field("admin_token", &"[REDACTED]")
            .finish()
    }
}

impl std::fmt::Debug for InstallationAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallationAccessToken")
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    #[test]
    fn session_debug_never_leaks_the_queue_token() {
        let secret = "queue-bearer-secret-value";
        let session = RunnerScaleSetSession {
            session_id: None,
            owner_name: Some("fleet-a".to_string()),
            message_queue_url: Some("https://example.invalid/queue".to_string()),
            message_queue_access_token: Some(secret.to_string()),
            statistics: None,
        };
        let rendered = format!("{session:?}");
        assert!(
            !rendered.contains(secret),
            "Debug output must not contain the queue access token: {rendered}"
        );
        assert!(rendered.contains("REDACTED"));
    }
}
