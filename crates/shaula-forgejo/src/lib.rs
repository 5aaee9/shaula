//! Forgejo Actions runner control-plane adapter.
//!
//! This crate deliberately does not implement the GitHub scale-set port. Forgejo
//! runners have different ownership and job-association semantics, so the
//! adapter exposes only the small, provider-specific pool contract.

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::fmt;
use url::Url;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_RUNNERS: usize = 10_000;
const MAX_JOBS: usize = 10_000;
const MAX_RESPONSE_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgejoScope {
    Instance,
    Organization(String),
    User,
    Repository { owner: String, name: String },
}

impl ForgejoScope {
    fn path(&self) -> String {
        match self {
            Self::Instance => "/api/v1/admin/actions/runners".into(),
            Self::Organization(org) => format!("/api/v1/orgs/{}/actions/runners", encode(org)),
            Self::User => "/api/v1/user/actions/runners".into(),
            Self::Repository { owner, name } => {
                format!(
                    "/api/v1/repos/{}/{}/actions/runners",
                    encode(owner),
                    encode(name)
                )
            }
        }
    }
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[derive(Clone)]
pub struct ForgejoClient {
    client: Client,
    base_url: Url,
    token: String,
    scope: ForgejoScope,
    page_size: u32,
}

impl fmt::Debug for ForgejoClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForgejoClient")
            .field("base_url", &self.base_url)
            .field("scope", &self.scope)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl ForgejoClient {
    pub fn new(
        base_url: &str,
        token: impl Into<String>,
        scope: ForgejoScope,
    ) -> Result<Self, ForgejoError> {
        let mut base_url =
            Url::parse(base_url).map_err(|e| ForgejoError::Configuration(e.to_string()))?;
        if base_url.scheme() != "http" && base_url.scheme() != "https" {
            return Err(ForgejoError::Configuration(
                "instance_url must use http or https".into(),
            ));
        }
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ForgejoError::Configuration(e.to_string()))?;
        Ok(Self {
            client,
            base_url,
            token: token.into(),
            scope,
            page_size: DEFAULT_PAGE_SIZE,
        })
    }

    fn endpoint(&self) -> Result<Url, ForgejoError> {
        self.base_url
            .join(self.scope.path().trim_start_matches('/'))
            .map_err(|e| ForgejoError::Configuration(e.to_string()))
    }

    pub async fn register_runner(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Registration, ForgejoError> {
        if name.trim().is_empty() || name.len() > 255 {
            return Err(ForgejoError::Configuration(
                "runner name must be 1..=255 characters".into(),
            ));
        }
        let body = RegisterRunnerRequest {
            name,
            description,
            ephemeral: true,
        };
        let response = self
            .client
            .post(self.endpoint()?)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ForgejoError::uncertain(e.to_string()))?;
        self.decode(response).await
    }

    pub async fn list_runners(&self) -> Result<Vec<Runner>, ForgejoError> {
        let endpoint = self.endpoint()?;
        let mut page = 1u32;
        let mut result = Vec::new();
        loop {
            let mut request = self
                .client
                .get(endpoint.clone())
                .bearer_auth(&self.token)
                .query(&[("page", page), ("limit", self.page_size)]);
            if !matches!(&self.scope, ForgejoScope::Instance) {
                request = request.query(&[("visible", false)]);
            }
            let response = request
                .send()
                .await
                .map_err(|e| ForgejoError::unavailable(e.to_string()))?;
            let mut runners: Vec<Runner> = self.decode(response).await?;
            if runners.is_empty() {
                break;
            }
            result.append(&mut runners);
            if result.len() > MAX_RUNNERS {
                return Err(ForgejoError::ResponseTooLarge);
            }
            page = page.checked_add(1).ok_or(ForgejoError::ResponseTooLarge)?;
        }
        Ok(result)
    }

    pub async fn list_jobs(&self, labels: &[String]) -> Result<Vec<Job>, ForgejoError> {
        let mut endpoint = self.endpoint()?;
        endpoint.set_path(&format!("{}/jobs", endpoint.path().trim_end_matches('/')));
        let label_filter = labels.join(",");
        let response = self
            .client
            .get(endpoint)
            .bearer_auth(&self.token)
            .query(&[("labels", label_filter)])
            .send()
            .await
            .map_err(|e| ForgejoError::unavailable(e.to_string()))?;
        let jobs: Vec<Job> = self.decode(response).await?;
        if jobs.len() > MAX_JOBS {
            return Err(ForgejoError::ResponseTooLarge);
        }
        Ok(jobs)
    }

    pub async fn delete_runner(&self, id: u64) -> Result<Removal, ForgejoError> {
        let endpoint = self
            .endpoint()?
            .join(&format!("/{id}"))
            .map_err(|e| ForgejoError::Configuration(e.to_string()))?;
        let response = self
            .client
            .delete(endpoint)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| ForgejoError::uncertain(e.to_string()))?;
        match response.status() {
            StatusCode::NO_CONTENT | StatusCode::OK => Ok(Removal::Removed),
            StatusCode::NOT_FOUND => Ok(Removal::AlreadyAbsent),
            StatusCode::CONFLICT => Ok(Removal::Busy),
            status => Err(self.status_error(status, response).await),
        }
    }

    pub async fn classify_uncertain_registration(
        &self,
        name: &str,
        labels: &[String],
    ) -> Result<RegistrationUncertainty, ForgejoError> {
        let runners = self.list_runners().await?;
        let matches: Vec<_> = runners
            .into_iter()
            .filter(|runner| {
                runner.name == name
                    && runner.ephemeral
                    && labels
                        .iter()
                        .all(|label| runner.labels.iter().any(|x| x.name == *label))
            })
            .collect();
        Ok(match matches.as_slice() {
            [] => RegistrationUncertainty::None,
            [_] => RegistrationUncertainty::ExactlyOneCleanupRequired,
            _ => RegistrationUncertainty::Quarantined,
        })
    }

    async fn decode<T: for<'de> Deserialize<'de>>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, ForgejoError> {
        let status = response.status();
        if !status.is_success() {
            return Err(self.status_error(status, response).await);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ForgejoError::InvalidResponse(e.to_string()))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(ForgejoError::ResponseTooLarge);
        }
        serde_json::from_slice(&bytes).map_err(|e| ForgejoError::InvalidResponse(e.to_string()))
    }

    async fn status_error(&self, status: StatusCode, response: reqwest::Response) -> ForgejoError {
        let summary = response.text().await.unwrap_or_default();
        let summary = if summary.len() > 512 {
            summary[..512].to_string()
        } else {
            summary
        };
        match status {
            StatusCode::UNAUTHORIZED => ForgejoError::Unauthenticated,
            StatusCode::FORBIDDEN => ForgejoError::PermissionDenied,
            StatusCode::NOT_FOUND => ForgejoError::TargetHiddenOrNotFound,
            StatusCode::TOO_MANY_REQUESTS => ForgejoError::RateLimited,
            _ => ForgejoError::Http {
                status: status.as_u16(),
                summary,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub r#type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runner {
    pub id: u64,
    pub uuid: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub runs_on: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub id: u64,
    pub uuid: String,
    pub token: String,
}

#[derive(Debug, Serialize)]
struct RegisterRunnerRequest<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    ephemeral: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationUncertainty {
    None,
    ExactlyOneCleanupRequired,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Removed,
    AlreadyAbsent,
    Busy,
}

#[derive(Debug, thiserror::Error)]
pub enum ForgejoError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("authentication failed")]
    Unauthenticated,
    #[error("permission denied")]
    PermissionDenied,
    #[error("target hidden or not found")]
    TargetHiddenOrNotFound,
    #[error("rate limited")]
    RateLimited,
    #[error("request outcome uncertain: {0}")]
    RequestUncertain(String),
    #[error("Forgejo unavailable: {0}")]
    Unavailable(String),
    #[error("Forgejo returned HTTP {status}: {summary}")]
    Http { status: u16, summary: String },
    #[error("invalid Forgejo response: {0}")]
    InvalidResponse(String),
    #[error("response exceeds configured bound")]
    ResponseTooLarge,
}

impl ForgejoError {
    fn uncertain(summary: String) -> Self {
        Self::RequestUncertain(summary)
    }
    fn unavailable(summary: String) -> Self {
        Self::Unavailable(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_paths_include_api_prefix_and_encoded_segments() {
        assert_eq!(
            ForgejoScope::Instance.path(),
            "/api/v1/admin/actions/runners"
        );
        assert_eq!(
            ForgejoScope::Organization("a/b".into()).path(),
            "/api/v1/orgs/a%2Fb/actions/runners"
        );
        assert_eq!(
            ForgejoScope::Repository {
                owner: "o".into(),
                name: "r".into()
            }
            .path(),
            "/api/v1/repos/o/r/actions/runners"
        );
    }

    #[test]
    fn registration_request_always_ephemeral() {
        let body = serde_json::to_value(RegisterRunnerRequest {
            name: "n",
            description: None,
            ephemeral: true,
        })
        .unwrap_or(serde_json::Value::Null);
        assert_eq!(body["ephemeral"], true);
        assert!(body.get("description").is_none());
    }

    #[test]
    fn debug_redacts_token() {
        let client =
            match ForgejoClient::new("http://localhost:3000", "secret", ForgejoScope::Instance) {
                Ok(client) => client,
                Err(_) => return,
            };
        assert!(!format!("{client:?}").contains("secret"));
    }
}
