//! Scoped runner inventory, association reads, pause and removal.

use reqwest::{Method, StatusCode};
use shaula_core::ports::gitlab::{GitlabRemovalOutcome, GitlabRunnerRef, GitlabRunningJob};
use std::collections::HashSet;

use crate::client::{GitlabClient, REQUEST_TIMEOUT};
use crate::error::GitlabError;
use crate::models::{RawJob, RawRunner, RawRunnerSummary};

const PAGE_SIZE: usize = 100;
const MAX_PAGES: u32 = 100;
const MAX_RUNNING_JOBS: usize = 100;

impl GitlabClient {
    /// Detailed registrations whose description starts with `prefix`. The
    /// summary list lacks `maintenance_note`, so matches are read one by one.
    pub async fn list_runners(&self, prefix: &str) -> Result<Vec<GitlabRunnerRef>, GitlabError> {
        let scope_id = self.scope_id().await?;
        let mut result = Vec::new();
        for summary in self.list_summaries(scope_id).await? {
            if !summary
                .description
                .as_deref()
                .is_some_and(|description| description.starts_with(prefix))
            {
                continue;
            }
            // Deleted between the list and the detail read: proven absent.
            if let Some(runner) = self.read_runner(scope_id, summary.id).await? {
                result.push(runner);
            }
        }
        Ok(result)
    }

    pub(crate) async fn runner_count(&self) -> Result<u64, GitlabError> {
        let scope_id = self.scope_id().await?;
        let count = self.list_summaries(scope_id).await?.len();
        Ok(u64::try_from(count).unwrap_or(u64::MAX))
    }

    async fn list_summaries(&self, scope_id: u64) -> Result<Vec<RawRunnerSummary>, GitlabError> {
        let scope_id = scope_id.to_string();
        let url = self.url(&["api", "v4", self.scope_collection(), &scope_id, "runners"])?;
        let started = std::time::Instant::now();
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for page in 1..=MAX_PAGES {
            if started.elapsed() >= REQUEST_TIMEOUT {
                break;
            }
            let response = self
                .rest(Method::GET, url.clone())
                .query(&[("type", self.scope_runner_type())])
                .query(&[("per_page", PAGE_SIZE), ("page", page as usize)])
                .send()
                .await
                .map_err(|error| GitlabError::unavailable(error.without_url().to_string()))?;
            let runners: Vec<RawRunnerSummary> = self.decode(response).await?;
            let last = runners.len() < PAGE_SIZE;
            for runner in runners {
                if !seen.insert(runner.id) {
                    return Err(GitlabError::unavailable(
                        "inventory changed during pagination",
                    ));
                }
                result.push(runner);
            }
            if last {
                return Ok(result);
            }
        }
        Err(GitlabError::unavailable(
            "inventory pagination budget exceeded",
        ))
    }

    /// `None` is absence: scope access is proven before the exact-id read.
    pub async fn get_runner(&self, id: u64) -> Result<Option<GitlabRunnerRef>, GitlabError> {
        let scope_id = self.scope_id().await?;
        self.read_runner(scope_id, id).await
    }

    async fn read_runner(
        &self,
        scope_id: u64,
        id: u64,
    ) -> Result<Option<GitlabRunnerRef>, GitlabError> {
        let id_segment = positive(id)?.to_string();
        let response = self
            .rest(
                Method::GET,
                self.url(&["api", "v4", "runners", &id_segment])?,
            )
            .send()
            .await
            .map_err(|error| GitlabError::unavailable(error.without_url().to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let runner: RawRunner = self.decode(response).await?;
        let owners = match self.scope_collection() {
            "groups" => &runner.groups,
            _ => &runner.projects,
        };
        if runner.id != id
            || runner.runner_type != self.scope_runner_type()
            || !owners.iter().any(|owner| owner.id == scope_id)
        {
            // Visible but outside the Fleet scope: neither ours nor absent.
            return Err(GitlabError::InvalidResponse(
                "runner is outside the configured scope".into(),
            ));
        }
        Ok(Some(GitlabRunnerRef {
            id: runner.id,
            description: runner.description.unwrap_or_default(),
            maintenance_note: runner.maintenance_note,
            status: runner.status,
            job_execution_status: runner.job_execution_status,
            paused: runner.paused,
            tags: runner.tag_list,
            run_untagged: runner.run_untagged,
        }))
    }

    pub async fn running_jobs(
        &self,
        runner_id: u64,
    ) -> Result<Option<Vec<GitlabRunningJob>>, GitlabError> {
        self.scope_id().await?;
        let id_segment = positive(runner_id)?.to_string();
        let response = self
            .rest(
                Method::GET,
                self.url(&["api", "v4", "runners", &id_segment, "jobs"])?,
            )
            .query(&[("status", "running")])
            .query(&[("per_page", MAX_RUNNING_JOBS)])
            .send()
            .await
            .map_err(|error| GitlabError::unavailable(error.without_url().to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let jobs: Vec<RawJob> = self.decode(response).await?;
        if jobs.len() >= MAX_RUNNING_JOBS {
            return Err(GitlabError::ResponseTooLarge);
        }
        Ok(Some(
            jobs.into_iter()
                .map(|job| GitlabRunningJob {
                    id: job.id,
                    project_id: job.project.id,
                    project_path: job.project.path_with_namespace,
                    pipeline_id: job.pipeline.id,
                    name: job.name,
                    status: job.status,
                    web_url: job.web_url,
                })
                .collect(),
        ))
    }

    /// Stops future pickups; an in-flight job continues.
    pub async fn pause_runner(&self, id: u64) -> Result<GitlabRemovalOutcome, GitlabError> {
        self.scope_id().await?;
        let id_segment = positive(id)?.to_string();
        let response = self
            .rest(
                Method::PUT,
                self.url(&["api", "v4", "runners", &id_segment])?,
            )
            .json(&serde_json::json!({ "paused": true }))
            .send()
            .await
            .map_err(|error| GitlabError::uncertain(error.without_url().to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(GitlabRemovalOutcome::AlreadyAbsent);
        }
        if response.status().is_server_error() {
            return Err(GitlabError::uncertain("runner pause outcome unknown"));
        }
        let runner: RawRunner = self.decode(response).await?;
        if runner.id != id || !runner.paused {
            return Err(GitlabError::uncertain("pause was not reflected"));
        }
        Ok(GitlabRemovalOutcome::Done)
    }

    /// Unregisters without interrupting an in-flight job.
    pub async fn delete_runner(&self, id: u64) -> Result<GitlabRemovalOutcome, GitlabError> {
        self.scope_id().await?;
        let id_segment = positive(id)?.to_string();
        let response = self
            .rest(
                Method::DELETE,
                self.url(&["api", "v4", "runners", &id_segment])?,
            )
            .send()
            .await
            .map_err(|error| GitlabError::uncertain(error.without_url().to_string()))?;
        match response.status() {
            StatusCode::NO_CONTENT | StatusCode::OK => Ok(GitlabRemovalOutcome::Done),
            StatusCode::NOT_FOUND => Ok(GitlabRemovalOutcome::AlreadyAbsent),
            status if status.is_server_error() => {
                Err(GitlabError::uncertain("runner deletion outcome unknown"))
            }
            _ => Err(self.status_error(response).await),
        }
    }
}

fn positive(id: u64) -> Result<u64, GitlabError> {
    if id == 0 {
        return Err(GitlabError::Configuration(
            "runner id must be positive".into(),
        ));
    }
    Ok(id)
}
