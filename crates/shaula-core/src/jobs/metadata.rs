//! Approved optional GitHub presentation fields, with no endpoint or credential bytes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobMetadata {
    pub owner_name: Option<String>,
    pub repository_name: Option<String>,
    pub job_display_name: Option<String>,
    pub job_workflow_ref: Option<String>,
    pub workflow_run_id: Option<i64>,
    pub event_name: Option<String>,
    pub queue_time: Option<chrono::DateTime<chrono::Utc>>,
    pub scale_set_assign_time: Option<chrono::DateTime<chrono::Utc>>,
    pub runner_assign_time: Option<chrono::DateTime<chrono::Utc>>,
    pub finish_time: Option<chrono::DateTime<chrono::Utc>>,
}

impl JobMetadata {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// A run link only; an opaque Scale Set job ID is never a REST job ID.
    pub fn github_run_url(&self) -> Option<String> {
        let target = crate::github::GitHubTarget::new_repository(
            self.owner_name.as_ref()?,
            self.repository_name.as_ref()?,
        )
        .ok()?;
        let run = self.workflow_run_id.filter(|id| *id > 0)?;
        Some(format!("{}/actions/runs/{run}", target.config_url()))
    }

    /// Conflicting identity metadata is retained in observations, never overwritten.
    pub fn identity_conflicts(&self, other: &Self) -> bool {
        conflict(&self.owner_name, &other.owner_name)
            || conflict(&self.repository_name, &other.repository_name)
            || conflict(&self.workflow_run_id, &other.workflow_run_id)
    }
}

fn conflict<T: Eq>(left: &Option<T>, right: &Option<T>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left != right)
}
