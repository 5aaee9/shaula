//! Exact job reads for the read-only Jobs projection (spec 0043 §6.1).

use reqwest::{Method, StatusCode};
use shaula_core::gitlab::GitlabScope;
use shaula_core::ports::gitlab::GitlabJobDetail;

use crate::client::GitlabClient;
use crate::error::GitlabError;
use crate::models::{RawJobDetail, RawProject};

impl GitlabClient {
    /// Proves the scope, then the project's place inside it, then reads the
    /// exact job. `None` is absence of the project or job after that proof.
    pub async fn job(
        &self,
        project_id: u64,
        job_id: u64,
    ) -> Result<Option<GitlabJobDetail>, GitlabError> {
        if project_id == 0 || job_id == 0 {
            return Err(GitlabError::Configuration(
                "job identity must be positive".into(),
            ));
        }
        let scope_id = self.scope_id().await?;
        let project_segment = project_id.to_string();
        let Some(project) = self
            .read_optional::<RawProject>(&["api", "v4", "projects", &project_segment])
            .await?
        else {
            return Ok(None);
        };
        let inside = match &self.target().scope {
            GitlabScope::Project { .. } => project_id == scope_id,
            GitlabScope::Group { path } => project
                .path_with_namespace
                .to_ascii_lowercase()
                .starts_with(&format!("{}/", path.to_ascii_lowercase())),
        };
        if project.id != project_id || !inside {
            return Err(GitlabError::InvalidResponse(
                "job project is outside the configured scope".into(),
            ));
        }
        let job_segment = job_id.to_string();
        let path = [
            "api",
            "v4",
            "projects",
            &project_segment,
            "jobs",
            &job_segment,
        ];
        let Some(job) = self.read_optional::<RawJobDetail>(&path).await? else {
            return Ok(None);
        };
        if job.id != job_id || job.pipeline.project_id != project_id {
            return Err(GitlabError::InvalidResponse(
                "job identity does not match the request".into(),
            ));
        }
        Ok(Some(GitlabJobDetail {
            id: job.id,
            project_id,
            project_path: project.path_with_namespace,
            pipeline_id: job.pipeline.id,
            name: job.name,
            status: job.status,
            runner_id: job.runner.map(|runner| runner.id),
        }))
    }

    async fn read_optional<T: serde::de::DeserializeOwned>(
        &self,
        path: &[&str],
    ) -> Result<Option<T>, GitlabError> {
        let response = self
            .rest(Method::GET, self.url(path)?)
            .send()
            .await
            .map_err(|error| GitlabError::unavailable(error.without_url().to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        self.decode(response).await.map(Some)
    }
}
