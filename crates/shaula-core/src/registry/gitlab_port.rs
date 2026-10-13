//! Durable GitLab Pool state (spec 0043): runner identities, the demand
//! anchor per Fleet revision, and Verified job associations. Separate from
//! `LifecycleStore` so the GitHub/Forgejo seams do not grow GitLab methods.

use async_trait::async_trait;

use crate::error::CoreResult;

/// Anchor lifecycle. `Registering` is written before the POST so a lost
/// response is classified by exact ownership lookup, never retried blindly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitlabAnchorState {
    Registering,
    Active,
    /// Superseded or decommissioned; deleted remotely before the row goes.
    Retiring,
    /// More than one exact candidate or contradictory evidence.
    Ambiguous,
}

impl GitlabAnchorState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registering => "Registering",
            Self::Active => "Active",
            Self::Retiring => "Retiring",
            Self::Ambiguous => "Ambiguous",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "Registering" => Self::Registering,
            "Active" => Self::Active,
            "Retiring" => Self::Retiring,
            "Ambiguous" => Self::Ambiguous,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabAnchorRow {
    pub fleet_key: String,
    pub fleet_revision: i64,
    pub anchor_name: String,
    pub runner_id: Option<i64>,
    pub state: GitlabAnchorState,
}

/// The job a Generation's registration acquired, persisted before pausing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitlabJobAssociation {
    pub job_id: i64,
    pub project_id: i64,
    pub pipeline_id: i64,
    pub observed_at: i64,
}

#[async_trait]
pub trait GitlabPoolStore: Send + Sync {
    /// Binds the exact registration id to a GitLab Generation; a different
    /// id for the same Generation is a conflict, never an overwrite.
    async fn generation_set_gitlab_runner(
        &self,
        id: &str,
        runner_id: i64,
        now: i64,
    ) -> CoreResult<()>;
    async fn generation_gitlab_runner(&self, id: &str) -> CoreResult<Option<i64>>;
    /// First association wins; a different job for the same Generation is a
    /// conflict (the registration was paused after its first job).
    async fn generation_set_gitlab_job(
        &self,
        id: &str,
        association: GitlabJobAssociation,
    ) -> CoreResult<()>;
    async fn generation_gitlab_job(&self, id: &str) -> CoreResult<Option<GitlabJobAssociation>>;
    async fn gitlab_anchors(&self, fleet_key: &str) -> CoreResult<Vec<GitlabAnchorRow>>;
    /// Inserts a `Registering` row unless one exists for this revision.
    async fn gitlab_anchor_begin(
        &self,
        fleet_key: &str,
        fleet_revision: i64,
        anchor_name: &str,
        now: i64,
    ) -> CoreResult<()>;
    async fn gitlab_anchor_set(
        &self,
        fleet_key: &str,
        fleet_revision: i64,
        runner_id: Option<i64>,
        state: GitlabAnchorState,
        now: i64,
    ) -> CoreResult<()>;
    async fn gitlab_anchor_remove(&self, fleet_key: &str, fleet_revision: i64) -> CoreResult<()>;
}
