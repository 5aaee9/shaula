//! Provider-specific pool port for GitLab runners (spec 0043).
//!
//! Independent of [`super::GitHubAccessPort`] and [`super::forgejo::ForgejoPoolPort`]:
//! GitLab demand needs an authorization proof, registrations are paused
//! before removal, and a registration's token can serve more than one process.

use async_trait::async_trait;

use crate::gitlab::{GitlabMatching, GitlabRegistrationRole};
use crate::secret::SecretString;

use super::{AccessFailure, EffectOutcome};

/// Single-registration material for `gitlab-runner run-single`. The token
/// reaches the runner only inside the protected `config.toml`.
#[derive(Clone, PartialEq, Eq)]
pub struct GitlabBootstrapMaterial {
    pub instance_url: String,
    pub runner_id: u64,
    pub runner_name: String,
    token: SecretString,
}

impl GitlabBootstrapMaterial {
    pub fn new(
        instance_url: impl Into<String>,
        runner_id: u64,
        runner_name: impl Into<String>,
        token: SecretString,
    ) -> Result<Self, &'static str> {
        let instance_url = instance_url.into();
        let runner_name = runner_name.into();
        let url = url::Url::parse(&instance_url)
            .map_err(|_| "GitLab bootstrap instance URL is invalid")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || runner_id == 0
            || runner_name.trim().is_empty()
            || runner_name.len() > 255
            || !toml_safe(&runner_name)
            || !toml_safe(&instance_url)
            || !token.expose().starts_with("glrt-")
            || token.expose().len() > 4096
            || !toml_safe(token.expose())
        {
            return Err("GitLab bootstrap material is incomplete or invalid");
        }
        Ok(Self {
            instance_url,
            runner_id,
            runner_name,
            token,
        })
    }

    /// Fixed upstream single-job arguments. No credential ever appears in argv.
    pub fn run_single_args(&self, config_path: &str, wait_timeout_secs: u64) -> Vec<String> {
        vec![
            "run-single".into(),
            "--config".into(),
            config_path.into(),
            "--runner".into(),
            self.runner_name.clone(),
            "--max-builds".into(),
            "1".into(),
            "--wait-timeout".into(),
            wait_timeout_secs.to_string(),
        ]
    }

    /// Credential-grade `config.toml` for the `shell` executor only.
    pub fn config_toml(&self) -> String {
        format!(
            "concurrent = 1\ncheck_interval = 3\n\n[[runners]]\n  name = \"{}\"\n  url = \"{}\"\n  id = {}\n  token = \"{}\"\n  executor = \"shell\"\n",
            self.runner_name,
            self.instance_url,
            self.runner_id,
            self.token.expose()
        )
    }

    /// Only for the protected worker hand-off and the `config.toml` render.
    pub fn token(&self) -> &str {
        self.token.expose()
    }

    pub fn identity(&self) -> crate::gitlab::GitlabBootstrapIdentity {
        crate::gitlab::GitlabBootstrapIdentity {
            instance_url: self.instance_url.clone(),
            runner_id: self.runner_id,
            runner_name: self.runner_name.clone(),
        }
    }
}

/// Values are rendered inside TOML basic strings, so quoting, escapes and
/// control characters are refused rather than escaped.
fn toml_safe(value: &str) -> bool {
    !value
        .chars()
        .any(|character| character.is_control() || matches!(character, '"' | '\\'))
}

impl std::fmt::Debug for GitlabBootstrapMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitlabBootstrapMaterial")
            .field("instance_url", &self.instance_url)
            .field("runner_id", &self.runner_id)
            .field("runner_name", &self.runner_name)
            .field("token", &"REDACTED")
            .finish()
    }
}

/// Registration request; the adapter adds scope, `locked` and `access_level`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabRunnerSpec {
    pub description: String,
    pub maintenance_note: String,
    pub role: GitlabRegistrationRole,
    pub matching: GitlabMatching,
}

#[derive(Clone, PartialEq, Eq)]
pub struct GitlabRegistration {
    pub id: u64,
    pub token: SecretString,
}

impl std::fmt::Debug for GitlabRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitlabRegistration")
            .field("id", &self.id)
            .field("token", &"REDACTED")
            .finish()
    }
}

/// Detailed registration as read from `GET /runners/:id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabRunnerRef {
    pub id: u64,
    pub description: String,
    pub maintenance_note: Option<String>,
    pub status: String,
    pub job_execution_status: Option<String>,
    pub paused: bool,
    pub tags: Vec<String>,
    pub run_untagged: bool,
}

impl GitlabRunnerRef {
    /// Unknown statuses prove neither online nor absent. GitLab reports a
    /// paused registration as `paused` regardless of contact state.
    pub fn is_known(&self) -> bool {
        matches!(
            self.status.as_str(),
            "online" | "offline" | "stale" | "never_contacted" | "paused"
        ) && matches!(
            self.job_execution_status.as_deref(),
            Some("idle" | "active")
        )
    }

    pub fn is_online_idle(&self) -> bool {
        self.status == "online" && self.job_execution_status.as_deref() == Some("idle")
    }

    pub fn is_active(&self) -> bool {
        self.job_execution_status.as_deref() == Some("active")
    }
}

/// A job executing on one registration; the `Verified` association evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabRunningJob {
    pub id: u64,
    pub project_id: u64,
    pub project_path: String,
    pub pipeline_id: u64,
    pub name: String,
    pub status: String,
    pub web_url: Option<String>,
}

/// An exact job read for the Jobs projection. It is never lifecycle evidence:
/// `runner_id` is `None` once the registration has been deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabJobDetail {
    pub id: u64,
    pub project_id: u64,
    pub project_path: String,
    pub pipeline_id: u64,
    pub name: String,
    pub status: String,
    pub runner_id: Option<u64>,
}

/// One `compatibleRunnerId` read with the anchor's same-request evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabAnchorDemand {
    pub pending: u64,
    pub anchor_present: bool,
    pub anchor_manageable: bool,
    pub anchor_paused: bool,
    pub anchor_matching: GitlabMatching,
}

impl GitlabAnchorDemand {
    /// Zero is only demand evidence when the filter itself is authorized and
    /// the anchor still carries the Fleet's matching attributes.
    pub fn proven_for(&self, matching: &GitlabMatching) -> Option<u64> {
        (self.anchor_present
            && self.anchor_manageable
            && self.anchor_paused
            && &self.anchor_matching == matching)
            .then_some(self.pending)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitlabRemovalOutcome {
    Done,
    AlreadyAbsent,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitlabAuthProbe {
    pub server_version: String,
    pub target_id: u64,
    pub checked_at_unix_ms: i64,
    pub valid_until_unix_ms: i64,
    pub runner_count: u64,
    #[serde(default)]
    pub token_expires_at: Option<String>,
}

#[async_trait]
pub trait GitlabPoolPort: Send + Sync {
    async fn register_runner(
        &self,
        spec: &GitlabRunnerSpec,
    ) -> Result<EffectOutcome<GitlabRegistration>, AccessFailure>;

    /// Detailed registrations in scope whose description starts with `prefix`.
    async fn list_runners(&self, prefix: &str) -> Result<Vec<GitlabRunnerRef>, AccessFailure>;

    /// `None` is absence only after the scope itself was proven readable.
    async fn get_runner(&self, id: u64) -> Result<Option<GitlabRunnerRef>, AccessFailure>;

    async fn anchor_demand(&self, anchor_id: u64) -> Result<GitlabAnchorDemand, AccessFailure>;

    /// `None` when the registration no longer exists.
    async fn running_jobs(
        &self,
        runner_id: u64,
    ) -> Result<Option<Vec<GitlabRunningJob>>, AccessFailure>;

    /// Exact `project_id/job_id` read inside the proven scope; `None` is
    /// absence (or loss of visibility) after the scope itself was proven.
    async fn job(
        &self,
        project_id: u64,
        job_id: u64,
    ) -> Result<Option<GitlabJobDetail>, AccessFailure>;

    async fn pause_runner(&self, id: u64) -> Result<GitlabRemovalOutcome, AccessFailure>;

    async fn delete_runner(&self, id: u64) -> Result<GitlabRemovalOutcome, AccessFailure>;

    async fn auth_probe(&self) -> Result<GitlabAuthProbe, AccessFailure>;
}
