//! v15+ task history exposes task.ID/task.Status, not a job-to-Runner proof.
use super::{ForgejoClient, ForgejoError, ForgejoScope};
use serde::Deserialize;
use shaula_core::jobs::{ForgejoTaskConclusion, ForgejoTaskResult};
use std::{collections::HashSet, time::Duration};

const HISTORY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PAGES: u32 = 10;

#[derive(Deserialize)]
struct Repository {
    id: u64,
    name: String,
    owner: Owner,
}
#[derive(Deserialize)]
struct Owner {
    id: u64,
    login: String,
}
#[derive(Deserialize)]
struct TaskPage {
    workflow_runs: Vec<Task>,
}
#[derive(Deserialize)]
struct Task {
    id: u64,
    status: String,
    run_number: u64,
    workflow_id: String,
}

impl ForgejoClient {
    /// Bounded optional enrichment. Missing tasks, partial pagination and unknown
    /// statuses produce no result; neither names nor a workflow's result substitute
    /// for the exact task ID retained from a runner-jobs snapshot.
    pub async fn task_results(
        &self,
        repository_id: u64,
        task_ids: &[u64],
    ) -> Result<Vec<ForgejoTaskResult>, ForgejoError> {
        if repository_id == 0 || task_ids.len() > 100 || task_ids.contains(&0) {
            return Err(ForgejoError::Configuration(
                "invalid task history lookup".into(),
            ));
        }
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        tokio::time::timeout(HISTORY_TIMEOUT, self.task_history(repository_id, task_ids))
            .await
            .map_err(|_| ForgejoError::unavailable("task history budget exceeded"))?
    }

    async fn task_history(
        &self,
        repository_id: u64,
        task_ids: &[u64],
    ) -> Result<Vec<ForgejoTaskResult>, ForgejoError> {
        self.verify_scope_identity().await?;
        // Numeric lookup avoids guessing repository names from jobs, handles or URLs.
        let endpoint = self
            .base_url
            .join(&format!("api/v1/repositories/{repository_id}"))?;
        let response = self
            .authorized(reqwest::Method::GET, endpoint)
            .send()
            .await
            .map_err(|e| ForgejoError::unavailable(e.to_string()))?;
        let repository: Repository = self.decode(response).await?;
        if repository.id != repository_id || repository.owner.id == 0 {
            return Err(ForgejoError::InvalidResponse(
                "repository identity mismatch".into(),
            ));
        }
        let scope_id = match self.expected_scope_id {
            Some(id) => Some(id),
            None => self.scope_identity().await?,
        };
        let in_scope = match &self.scope {
            ForgejoScope::Instance => true,
            ForgejoScope::Repository { .. } => scope_id == Some(repository.id),
            ForgejoScope::Organization(_) | ForgejoScope::User => {
                scope_id == Some(repository.owner.id)
            }
        };
        if !in_scope {
            return Err(ForgejoError::PermissionDenied);
        }
        let target = ForgejoScope::Repository {
            owner: repository.owner.login.clone(),
            name: repository.name.clone(),
        };
        target
            .validate()
            .map_err(|_| ForgejoError::InvalidResponse("invalid repository path".into()))?;
        let route = target.api_path();
        let repo_path = route.trim_end_matches("/actions/runners");
        let endpoint = self.base_url.join(&format!(
            "{}/actions/tasks",
            repo_path.trim_start_matches('/')
        ))?;
        let run_path = repo_path.trim_start_matches("/api/v1/repos/");
        let wanted: HashSet<_> = task_ids.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        let limit = self.page_size.min(100);
        for page in 1..=MAX_PAGES {
            let response = self
                .authorized(reqwest::Method::GET, endpoint.clone())
                .query(&[("page", page), ("limit", limit)])
                .send()
                .await
                .map_err(|e| ForgejoError::unavailable(e.to_string()))?;
            let tasks: TaskPage = self.decode(response).await?;
            if tasks.workflow_runs.len() > limit as usize {
                return Err(ForgejoError::ResponseTooLarge);
            }
            let last_page = tasks.workflow_runs.len() < limit as usize;
            for task in tasks.workflow_runs {
                if task.id == 0 || !seen.insert(task.id) {
                    return Err(ForgejoError::InvalidResponse(
                        "task history changed during pagination".into(),
                    ));
                }
                if !wanted.contains(&task.id) {
                    continue;
                }
                let conclusion = match task.status.as_str() {
                    "success" => ForgejoTaskConclusion::Success,
                    "failure" => ForgejoTaskConclusion::Failure,
                    "cancelled" => ForgejoTaskConclusion::Cancelled,
                    "skipped" => ForgejoTaskConclusion::Skipped,
                    _ => continue,
                };
                if task.run_number == 0
                    || task.workflow_id.len() > 1024
                    || task.workflow_id.chars().any(char::is_control)
                {
                    return Err(ForgejoError::InvalidResponse(
                        "invalid task history metadata".into(),
                    ));
                }
                result.push(ForgejoTaskResult {
                    repository_id,
                    task_id: task.id,
                    conclusion,
                    owner: repository.owner.login.clone(),
                    repository: repository.name.clone(),
                    run_number: task.run_number,
                    run_url: self
                        .base_url
                        .join(&format!("{run_path}/actions/runs/{}", task.run_number))?
                        .to_string(),
                    workflow: task.workflow_id,
                });
            }
            if last_page || result.len() == wanted.len() {
                break;
            }
        }
        Ok(result)
    }
}
