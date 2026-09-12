//! Provider-specific pool port for Forgejo Actions.
//!
//! This port is deliberately independent of [`super::GitHubAccessPort`]:
//! Forgejo has no scale set, runner group, message session, or verified job
//! assignment.

use async_trait::async_trait;

use crate::secret::SecretString;

use super::{AccessFailure, EffectOutcome};

/// One-shot Forgejo registration material. The token never belongs in the
/// Terraform input envelope; template runtimes consume it only through a
/// protected bootstrap file/Secret.
#[derive(Clone, PartialEq, Eq)]
pub struct ForgejoBootstrapMaterial {
    pub instance_url: String,
    pub uuid: String,
    token: SecretString,
    pub labels: Vec<String>,
}

impl ForgejoBootstrapMaterial {
    pub fn new(
        instance_url: impl Into<String>,
        uuid: impl Into<String>,
        token: SecretString,
        labels: Vec<String>,
    ) -> Result<Self, &'static str> {
        let instance_url = instance_url.into();
        let uuid = uuid.into();
        let url = url::Url::parse(&instance_url)
            .map_err(|_| "Forgejo bootstrap instance URL is invalid")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || uuid.trim().is_empty()
            || uuid.len() > 128
            || uuid.chars().any(char::is_control)
            || token.expose().is_empty()
            || token.expose().len() > 4096
            || token.expose().chars().any(char::is_control)
        {
            return Err("Forgejo bootstrap material is incomplete or invalid");
        }
        if labels.len() > 32
            || labels.iter().any(|label| {
                label.trim().is_empty() || label.len() > 255 || label.chars().any(char::is_control)
            })
        {
            return Err("Forgejo bootstrap labels are incomplete or invalid");
        }
        Ok(Self {
            instance_url,
            uuid,
            token,
            labels,
        })
    }

    /// Fixed upstream one-job arguments. No credential ever appears in argv.
    pub fn one_job_args(&self) -> Vec<String> {
        let mut args = vec![
            "one-job".into(),
            "--url".into(),
            self.instance_url.clone(),
            "--uuid".into(),
            self.uuid.clone(),
            "--token-url".into(),
            "file:///data/.forgejo-token".into(),
        ];
        for label in &self.labels {
            args.extend(["--label".into(), label.clone()]);
        }
        args.push("--wait".into());
        args
    }

    pub fn token(&self) -> &str {
        self.token.expose()
    }

    pub fn identity(&self) -> crate::forgejo::ForgejoBootstrapIdentity {
        crate::forgejo::ForgejoBootstrapIdentity {
            instance_url: self.instance_url.clone(),
            uuid: self.uuid.clone(),
            labels: self.labels.clone(),
        }
    }
}

impl std::fmt::Debug for ForgejoBootstrapMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgejoBootstrapMaterial")
            .field("instance_url", &self.instance_url)
            .field("uuid", &self.uuid)
            .field("token", &"REDACTED")
            .field("labels", &self.labels)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgejoRunnerRef {
    pub id: u64,
    pub uuid: String,
    pub name: String,
    pub status: String,
    pub labels: Vec<String>,
    pub ephemeral: bool,
    pub version: Option<String>,
}

impl ForgejoRunnerRef {
    pub fn is_idle(&self) -> bool {
        self.status == "idle"
    }

    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// Unknown statuses are intentionally not considered safe or online.
    pub fn is_known(&self) -> bool {
        matches!(self.status.as_str(), "offline" | "idle" | "active")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgejoJob {
    pub id: u64,
    pub handle: String,
    pub attempt: u64,
    pub status: String,
    pub runs_on: Vec<String>,
    pub task_id: u64,
    pub run_id: u64,
    pub repo_id: u64,
    pub name: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ForgejoRegistration {
    pub id: u64,
    pub uuid: String,
    pub token: SecretString,
}

impl std::fmt::Debug for ForgejoRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgejoRegistration")
            .field("id", &self.id)
            .field("uuid", &self.uuid)
            .field("token", &"REDACTED")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgejoRegistrationUncertainty {
    None,
    ExactlyOneCleanupRequired,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgejoRemovalOutcome {
    Removed,
    AlreadyAbsent,
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgejoDemandSnapshot {
    pub waiting_jobs: u64,
    pub running_jobs: u64,
    pub observed_at_unix_ms: i64,
    pub stale: bool,
}

impl ForgejoDemandSnapshot {
    pub fn target(&self, min_runners: u64, max_runners: u64) -> u64 {
        min_runners
            .saturating_add(self.waiting_jobs)
            .min(max_runners)
    }

    /// Builds a replacement snapshot from one jobs response. Running jobs
    /// are observed for diagnostics but never increase pool demand.
    pub fn from_jobs<I>(jobs: I, observed_at_unix_ms: i64) -> Self
    where
        I: IntoIterator<Item = ForgejoJob>,
    {
        let mut snapshot = Self {
            waiting_jobs: 0,
            running_jobs: 0,
            observed_at_unix_ms,
            stale: false,
        };
        for job in jobs {
            match job.status.as_str() {
                "waiting" => snapshot.waiting_jobs = snapshot.waiting_jobs.saturating_add(1),
                "running" => snapshot.running_jobs = snapshot.running_jobs.saturating_add(1),
                _ => {}
            }
        }
        snapshot
    }

    /// Keeps the last known values while marking the observation stale. A
    /// failed poll therefore cannot look like zero demand.
    pub fn stale_from(previous: &Self, observed_at_unix_ms: i64) -> Self {
        Self {
            waiting_jobs: previous.waiting_jobs,
            running_jobs: previous.running_jobs,
            observed_at_unix_ms,
            stale: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgejoTemplateEvidence {
    /// No task is held AND task acquisition is fenced until registration
    /// removal. A momentary idle process/metric/log snapshot is insufficient.
    ProcessIdle,
    ProcessHoldsTask,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgejoRemovalDecision {
    AlreadyAbsent,
    SafeToDelete,
    DeferBusy,
}

pub fn classify_removal(
    runner: Option<&ForgejoRunnerRef>,
    evidence: ForgejoTemplateEvidence,
) -> ForgejoRemovalDecision {
    let Some(runner) = runner else {
        return ForgejoRemovalDecision::AlreadyAbsent;
    };
    if runner.is_idle() && matches!(evidence, ForgejoTemplateEvidence::ProcessIdle) {
        ForgejoRemovalDecision::SafeToDelete
    } else {
        ForgejoRemovalDecision::DeferBusy
    }
}

#[async_trait]
pub trait ForgejoPoolPort: Send + Sync {
    async fn register_runner(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<EffectOutcome<ForgejoRegistration>, AccessFailure>;

    async fn list_runners(&self) -> Result<Vec<ForgejoRunnerRef>, AccessFailure>;

    /// Exact-ID confirmation after a successful scope read. Paginated inventory
    /// is sorted by mutable activity and alone cannot prove an ID absent.
    async fn get_runner(&self, _id: u64) -> Result<Option<ForgejoRunnerRef>, AccessFailure> {
        Err(AccessFailure::Unavailable {
            summary: "exact Forgejo inventory lookup unavailable".into(),
        })
    }

    async fn list_jobs(&self, labels: &[String]) -> Result<Vec<ForgejoJob>, AccessFailure>;

    async fn delete_runner(&self, id: u64) -> Result<ForgejoRemovalOutcome, AccessFailure>;

    async fn classify_uncertain_registration(
        &self,
        name: &str,
        labels: &[String],
    ) -> Result<ForgejoRegistrationUncertainty, AccessFailure>;

    /// Returns only exact, jointly evidenced candidates for cleanup after an
    /// uncertain registration response.  The default keeps existing test and
    /// adapter implementations source-compatible; adapters that can expose
    /// the candidate identity should override it.
    async fn uncertain_registration_candidates(
        &self,
        _name: &str,
        _labels: &[String],
    ) -> Result<Vec<ForgejoRunnerRef>, AccessFailure> {
        Ok(Vec::new())
    }

    async fn auth_probe(&self) -> Result<ForgejoAuthProbe, AccessFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ForgejoAuthProbe {
    /// Server version checked with the scope probe. Empty only in historical metadata.
    #[serde(default)]
    pub server_version: String,
    /// User-scope routing is principal-relative; rotations must retain this ID.
    pub principal_id: Option<u64>,
    /// Immutable organization/repository ID; names alone do not survive rename/reuse.
    pub target_id: Option<u64>,
    pub checked_at_unix_ms: i64,
    pub valid_until_unix_ms: i64,
    pub runner_count: u64,
}

#[cfg(test)]
#[path = "forgejo_tests.rs"]
mod tests;
