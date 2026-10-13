use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use shaula_core::gitlab::{validate_tags, GitlabRegistrationRole, GitlabScope, GitlabTarget};
use shaula_core::ports::gitlab::{GitlabRegistration, GitlabRunnerSpec};
use shaula_core::secret::SecretString;
use std::{fmt, time::Duration};
use url::Url;

use crate::body::{read_body, truncate_chars};
use crate::error::GitlabError;
use crate::models::{RawGroup, RawProject, RawRegistration, RawVersion, RegisterRunnerRequest};

pub(crate) const MAX_RESPONSE_BYTES: usize = 1_048_576;
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct GitlabClient {
    client: Client,
    base_url: Url,
    token: SecretString,
    target: GitlabTarget,
    expected_scope_id: Option<u64>,
}

impl fmt::Debug for GitlabClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitlabClient")
            .field("base_url", &self.base_url)
            .field("target", &self.target)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl GitlabClient {
    pub fn new(target: GitlabTarget, token: SecretString) -> Result<Self, GitlabError> {
        target
            .validate()
            .map_err(|error| GitlabError::Configuration(error.to_string()))?;
        let mut base_url = Url::parse(&target.instance_url)?;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let raw = token.expose();
        if raw.trim().is_empty() || raw.len() > 4096 || raw.chars().any(char::is_control) {
            return Err(GitlabError::Configuration(
                "token must contain 1..=4096 bytes without control characters".into(),
            ));
        }
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| GitlabError::Configuration(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            token,
            target,
            expected_scope_id: None,
        })
    }

    /// Pins the scope path to the numeric id captured at validation time.
    pub fn with_expected_scope_id(mut self, id: u64) -> Result<Self, GitlabError> {
        if id == 0 {
            return Err(GitlabError::Configuration(
                "scope identity must be positive".into(),
            ));
        }
        self.expected_scope_id = Some(id);
        Ok(self)
    }

    pub fn target(&self) -> &GitlabTarget {
        &self.target
    }

    /// `segments` are percent-encoded individually, so a full path such as
    /// `group/project` becomes the single `group%2Fproject` segment.
    pub(crate) fn url(&self, segments: &[&str]) -> Result<Url, GitlabError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| GitlabError::Configuration("instance_url cannot be a base".into()))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    pub(crate) fn rest(&self, method: Method, url: Url) -> RequestBuilder {
        self.client
            .request(method, url)
            .header("PRIVATE-TOKEN", self.token.expose())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub(crate) fn graphql(&self, url: Url) -> RequestBuilder {
        self.client
            .post(url)
            .bearer_auth(self.token.expose())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    /// Resolves the scope path and proves it is still the pinned group/project.
    pub(crate) async fn scope_id(&self) -> Result<u64, GitlabError> {
        let (id, path) = match &self.target.scope {
            GitlabScope::Group { path } => {
                let group: RawGroup = self.get(&["api", "v4", "groups", path]).await?;
                (group.id, group.full_path)
            }
            GitlabScope::Project { path } => {
                let project: RawProject = self.get(&["api", "v4", "projects", path]).await?;
                (project.id, project.path_with_namespace)
            }
        };
        if id == 0
            || !path.eq_ignore_ascii_case(self.target.scope.path())
            || self
                .expected_scope_id
                .is_some_and(|expected| expected != id)
        {
            return Err(GitlabError::PermissionDenied);
        }
        Ok(id)
    }

    pub(crate) fn scope_runner_type(&self) -> &'static str {
        match self.target.scope {
            GitlabScope::Group { .. } => "group_type",
            GitlabScope::Project { .. } => "project_type",
        }
    }

    pub(crate) fn scope_collection(&self) -> &'static str {
        match self.target.scope {
            GitlabScope::Group { .. } => "groups",
            GitlabScope::Project { .. } => "projects",
        }
    }

    pub async fn server_version(&self) -> Result<String, GitlabError> {
        let version: RawVersion = self.get(&["api", "v4", "version"]).await?;
        if !shaula_core::gitlab::supports_server_version(&version.version) {
            return Err(GitlabError::UnsupportedServerVersion);
        }
        Ok(version.version)
    }

    pub async fn register_runner(
        &self,
        spec: &GitlabRunnerSpec,
    ) -> Result<GitlabRegistration, GitlabError> {
        if spec.description.trim().is_empty()
            || spec.description.len() > 255
            || spec.description.chars().any(char::is_control)
            || spec.maintenance_note.len() > 1024
            || spec.maintenance_note.chars().any(char::is_control)
            || (spec.matching.tags.is_empty() && !spec.matching.run_untagged)
        {
            return Err(GitlabError::Configuration(
                "runner description, note, or matching exceeds registration limits".into(),
            ));
        }
        validate_tags(&spec.matching.tags)
            .map_err(|error| GitlabError::Configuration(error.to_string()))?;
        let scope_id = self.scope_id().await?;
        self.server_version().await?;
        let group = matches!(self.target.scope, GitlabScope::Group { .. });
        let body = RegisterRunnerRequest {
            runner_type: self.scope_runner_type(),
            group_id: group.then_some(scope_id),
            project_id: (!group).then_some(scope_id),
            description: &spec.description,
            maintenance_note: &spec.maintenance_note,
            tag_list: &spec.matching.tags,
            run_untagged: spec.matching.run_untagged,
            locked: true,
            paused: spec.role == GitlabRegistrationRole::Anchor,
            access_level: "not_protected",
        };
        let response = self
            .rest(Method::POST, self.url(&["api", "v4", "user", "runners"])?)
            .json(&body)
            .send()
            .await
            .map_err(|error| GitlabError::uncertain(error.without_url().to_string()))?;
        if response.status() != StatusCode::CREATED {
            // Never project a registration body: it may carry a runner token.
            return Err(match response.status() {
                status if status.is_success() || status.is_server_error() => {
                    GitlabError::uncertain("unexpected registration response status")
                }
                StatusCode::UNAUTHORIZED => GitlabError::Unauthenticated,
                StatusCode::FORBIDDEN => GitlabError::PermissionDenied,
                StatusCode::NOT_FOUND => GitlabError::TargetHiddenOrNotFound,
                StatusCode::TOO_MANY_REQUESTS => GitlabError::RateLimited,
                _ => GitlabError::uncertain("registration request was not accepted"),
            });
        }
        let registration: RawRegistration = self.decode_body(response).await.map_err(|_| {
            GitlabError::uncertain("registration succeeded but its response was unreadable")
        })?;
        if registration.id == 0 || !registration.token.starts_with("glrt-") {
            return Err(GitlabError::uncertain(
                "registration response is missing an id or runner token",
            ));
        }
        Ok(GitlabRegistration {
            id: registration.id,
            token: SecretString::new(registration.token),
        })
    }

    pub(crate) async fn get<T: DeserializeOwned>(&self, path: &[&str]) -> Result<T, GitlabError> {
        let response = self
            .rest(Method::GET, self.url(path)?)
            .send()
            .await
            .map_err(|error| GitlabError::unavailable(error.without_url().to_string()))?;
        self.decode(response).await
    }

    pub(crate) async fn decode<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, GitlabError> {
        if !response.status().is_success() {
            return Err(self.status_error(response).await);
        }
        self.decode_body(response).await
    }

    pub(crate) async fn decode_body<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, GitlabError> {
        let bytes = read_body(response).await?;
        serde_json::from_slice(&bytes)
            .map_err(|error| GitlabError::InvalidResponse(error.to_string()))
    }

    pub(crate) async fn status_error(&self, response: Response) -> GitlabError {
        let status = response.status();
        match status {
            StatusCode::UNAUTHORIZED => GitlabError::Unauthenticated,
            StatusCode::FORBIDDEN => GitlabError::PermissionDenied,
            StatusCode::NOT_FOUND => GitlabError::TargetHiddenOrNotFound,
            StatusCode::TOO_MANY_REQUESTS => GitlabError::RateLimited,
            _ => GitlabError::Http {
                status: status.as_u16(),
                summary: self.response_summary(response).await,
            },
        }
    }

    async fn response_summary(&self, response: Response) -> String {
        match read_body(response).await {
            Ok(bytes) => {
                let body = String::from_utf8_lossy(&bytes);
                truncate_chars(&body.replace(self.token.expose(), "[REDACTED]"), 512)
            }
            Err(GitlabError::ResponseTooLarge) => "error response exceeds configured bound".into(),
            Err(_) => "unable to read error response".into(),
        }
    }
}
