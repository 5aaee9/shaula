//! Exact repository/task evidence; this never proves a Generation association.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgejoTaskConclusion {
    Success,
    Failure,
    Cancelled,
    Skipped,
}

impl ForgejoTaskConclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}

/// Returned only for exact task IDs seen in a scoped runner-jobs snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgejoTaskResult {
    pub repository_id: u64,
    pub task_id: u64,
    pub conclusion: ForgejoTaskConclusion,
    pub owner: String,
    pub repository: String,
    pub run_number: u64,
    /// Constructed beneath the configured instance, never a response-supplied URL.
    pub run_url: String,
    pub workflow: String,
}

impl ForgejoTaskResult {
    pub fn validate_for_instance(&self, instance: &str) -> Result<(), &'static str> {
        let valid_url = url::Url::parse(&self.run_url)
            .ok()
            .zip(url::Url::parse(instance).ok())
            .is_some_and(|(url, base)| {
                matches!(url.scheme(), "http" | "https")
                    && url.origin() == base.origin()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url
                        .path()
                        .starts_with(&format!("{}/", base.path().trim_end_matches('/')))
            });
        if !valid_url
            || self.run_number == 0
            || [&self.owner, &self.repository, &self.workflow, &self.run_url]
                .iter()
                .any(|value| {
                    value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control)
                })
        {
            return Err("invalid Forgejo task result metadata");
        }
        Ok(())
    }
}

/// Captured local job identity. The store rechecks its task and scope at commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgejoJobLookup {
    pub record_id: String,
    pub repository_id: u64,
    pub task_id: u64,
}

/// Public retained proof; decimal IDs stay exact in browser clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgejoJobResult {
    pub task_id: String,
    pub conclusion: ForgejoTaskConclusion,
    pub observed_at: i64,
    pub run_number: String,
    pub run_url: String,
    pub workflow: String,
}
