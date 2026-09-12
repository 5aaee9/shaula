use reqwest::{Client, Response, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use shaula_core::secret::SecretString;
use std::{collections::HashSet, fmt, time::Duration};
use url::Url;

use crate::body::{read_body, truncate_chars};
use crate::error::ForgejoError;
pub(crate) use crate::labels::{normalize_label_names, normalize_runner_labels};
use crate::models::{ForgejoScope, Job, Registration, Removal, Runner, RunnerBootstrapMaterial};

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_RUNNERS: usize = 10_000;
const MAX_JOBS: usize = 10_000;
pub(crate) const MAX_RESPONSE_BYTES: usize = 1_048_576;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct ForgejoClient {
    client: Client,
    base_url: Url,
    token: SecretString,
    scope: ForgejoScope,
    page_size: u32,
    expected_scope_id: Option<u64>,
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
        scope
            .validate()
            .map_err(|message| ForgejoError::Configuration(message.into()))?;

        let mut base_url = Url::parse(base_url)?;
        if base_url.scheme() != "http" && base_url.scheme() != "https" {
            return Err(ForgejoError::Configuration(
                "instance_url must use http or https".into(),
            ));
        }
        if base_url.host_str().is_none_or(|host| host.is_empty()) {
            return Err(ForgejoError::Configuration(
                "instance_url must include a host".into(),
            ));
        }
        if !base_url.username().is_empty() || base_url.password().is_some() {
            return Err(ForgejoError::Configuration(
                "instance_url must not include user information".into(),
            ));
        }
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(ForgejoError::Configuration(
                "instance_url must not include a query or fragment".into(),
            ));
        }
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }

        let token = token.into();
        if token.trim().is_empty() || token.len() > 4096 || token.chars().any(char::is_control) {
            return Err(ForgejoError::Configuration(
                "token must contain 1..=4096 bytes without control characters".into(),
            ));
        }

        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| ForgejoError::Configuration(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            token: SecretString::new(token),
            scope,
            page_size: DEFAULT_PAGE_SIZE,
            expected_scope_id: None,
        })
    }

    /// Overrides the page size used for inventory reads.
    pub fn instance_url(&self) -> &Url {
        &self.base_url
    }

    pub fn scope(&self) -> &ForgejoScope {
        &self.scope
    }

    /// Turns a successful registration into the exact material consumed by
    /// `forgejo-runner one-job --token-url file:...`.
    pub fn bootstrap_material(
        &self,
        registration: &Registration,
        labels: &[String],
    ) -> Result<RunnerBootstrapMaterial, ForgejoError> {
        Ok(RunnerBootstrapMaterial {
            instance_url: self.base_url.clone(),
            uuid: registration.uuid.clone(),
            token: registration.token.clone(),
            labels: normalize_runner_labels(labels)?,
        })
    }

    pub fn with_page_size(mut self, page_size: u32) -> Result<Self, ForgejoError> {
        if page_size == 0 || page_size > 1_000 {
            return Err(ForgejoError::Configuration(
                "page_size must be in 1..=1000".into(),
            ));
        }
        self.page_size = page_size;
        Ok(self)
    }

    fn endpoint(&self) -> Result<Url, ForgejoError> {
        self.base_url
            .join(self.scope.api_path().trim_start_matches('/'))
            .map_err(ForgejoError::from)
    }

    fn endpoint_with_suffix(&self, suffix: &str) -> Result<Url, ForgejoError> {
        let mut endpoint = self.endpoint()?;
        let path = format!(
            "{}/{}",
            endpoint.path().trim_end_matches('/'),
            suffix.trim_start_matches('/')
        );
        endpoint.set_path(&path);
        endpoint.set_query(None);
        Ok(endpoint)
    }

    fn authorized(&self, method: reqwest::Method, endpoint: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, endpoint)
            .bearer_auth(self.token.expose())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub async fn register_runner(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Registration, ForgejoError> {
        if name.trim().is_empty()
            || name.len() > 255
            || name.chars().any(char::is_control)
            || description
                .is_some_and(|text| text.len() > 1024 || text.chars().any(char::is_control))
        {
            return Err(ForgejoError::Configuration(
                "runner name or description exceeds registration limits".into(),
            ));
        }
        self.verify_scope_identity().await?;
        self.server_version().await?;
        let body = RegisterRunnerRequest {
            name,
            description,
            ephemeral: true,
        };
        let response = self
            .authorized(reqwest::Method::POST, self.endpoint()?)
            .json(&body)
            .send()
            .await
            .map_err(|error| ForgejoError::uncertain(error.to_string()))?;
        if response.status() != StatusCode::CREATED {
            return Err(
                if response.status().is_success() || response.status().is_server_error() {
                    // The effect may have landed. Never project the response body:
                    // a nonconforming proxy may have included the one-shot token.
                    ForgejoError::uncertain("unexpected registration response status")
                } else {
                    self.status_error(response).await
                },
            );
        }
        let registration: RawRegistration = self.decode_body(response).await.map_err(|_| {
            ForgejoError::uncertain("registration succeeded but its response was unreadable")
        })?;
        if registration.id == 0
            || registration.uuid.trim().is_empty()
            || registration.token.trim().is_empty()
        {
            return Err(ForgejoError::uncertain(
                "registration response is missing id, uuid, or token",
            ));
        }
        Ok(Registration::new(
            registration.id,
            registration.uuid,
            registration.token,
        ))
    }

    pub async fn list_runners(&self) -> Result<Vec<Runner>, ForgejoError> {
        self.verify_scope_identity().await?;
        let endpoint = self.endpoint()?;
        let mut page = 1u32;
        let started = std::time::Instant::now();
        let mut result = Vec::new();
        let mut seen_ids = HashSet::new();
        loop {
            if page > 256 || started.elapsed() >= REQUEST_TIMEOUT {
                return Err(ForgejoError::unavailable(
                    "inventory pagination budget exceeded",
                ));
            }
            let mut request = self
                .authorized(reqwest::Method::GET, endpoint.clone())
                .query(&[("page", page), ("limit", self.page_size)]);
            if !matches!(&self.scope, ForgejoScope::Instance) {
                request = request.query(&[("visible", false)]);
            }
            let response = request
                .send()
                .await
                .map_err(|error| ForgejoError::unavailable(error.to_string()))?;
            let runners: Vec<Runner> = self.decode(response).await?;
            if runners.is_empty() {
                break;
            }
            for runner in runners {
                if !seen_ids.insert(runner.id) {
                    return Err(ForgejoError::unavailable(
                        "inventory changed during pagination",
                    ));
                }
                result.push(runner);
            }
            if result.len() > MAX_RUNNERS {
                return Err(ForgejoError::ResponseTooLarge);
            }
            page = page.checked_add(1).ok_or(ForgejoError::ResponseTooLarge)?;
        }
        Ok(result)
    }

    /// A scoped exact-ID read; callers must separately establish scope access
    /// before treating 404 as absence rather than a hidden target.
    pub async fn get_runner(&self, id: u64) -> Result<Option<Runner>, ForgejoError> {
        self.verify_scope_identity().await?;
        if id == 0 {
            return Err(ForgejoError::Configuration(
                "runner ID must be positive".into(),
            ));
        }
        let response = self
            .authorized(
                reqwest::Method::GET,
                self.endpoint_with_suffix(&id.to_string())?,
            )
            .send()
            .await
            .map_err(|error| ForgejoError::unavailable(error.to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        self.decode(response).await.map(Some)
    }

    pub async fn list_jobs(&self, labels: &[String]) -> Result<Vec<Job>, ForgejoError> {
        self.verify_scope_identity().await?;
        let endpoint = self.endpoint_with_suffix("jobs")?;
        let labels = normalize_label_names(labels)?;
        let label_filter = labels.join(",");
        let response = self
            .authorized(reqwest::Method::GET, endpoint)
            .query(&[("labels", label_filter)])
            .send()
            .await
            .map_err(|error| ForgejoError::unavailable(error.to_string()))?;
        let jobs: Option<Vec<Job>> = self.decode(response).await?;
        let jobs = jobs.unwrap_or_default();
        if jobs.len() > MAX_JOBS {
            return Err(ForgejoError::ResponseTooLarge);
        }
        Ok(jobs)
    }

    pub async fn delete_runner(&self, id: u64) -> Result<Removal, ForgejoError> {
        self.verify_scope_identity().await?;
        if id == 0 {
            return Err(ForgejoError::Configuration(
                "runner id must be positive".into(),
            ));
        }
        let response = self
            .authorized(
                reqwest::Method::DELETE,
                self.endpoint_with_suffix(&id.to_string())?,
            )
            .send()
            .await
            .map_err(|error| ForgejoError::uncertain(error.to_string()))?;
        match response.status() {
            StatusCode::NO_CONTENT | StatusCode::OK => Ok(Removal::Removed),
            StatusCode::NOT_FOUND => Ok(Removal::AlreadyAbsent),
            // Forgejo currently deletes directly and does not promise a busy
            // protection. Keep this conservative outcome for installations or
            // proxies that reject a deletion while local evidence is busy.
            StatusCode::CONFLICT => Ok(Removal::Busy),
            _ => Err(self.status_error(response).await),
        }
    }

    async fn decode<T: DeserializeOwned>(&self, response: Response) -> Result<T, ForgejoError> {
        if !response.status().is_success() {
            return Err(self.status_error(response).await);
        }
        self.decode_body(response).await
    }

    async fn decode_body<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, ForgejoError> {
        let bytes = read_body(response).await?;
        serde_json::from_slice(&bytes)
            .map_err(|error| ForgejoError::InvalidResponse(error.to_string()))
    }

    async fn status_error(&self, response: Response) -> ForgejoError {
        let status = response.status();
        let summary = self.response_summary(response).await;
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

    async fn response_summary(&self, response: Response) -> String {
        match read_body(response).await {
            Ok(bytes) => {
                let body = String::from_utf8_lossy(&bytes);
                let body = body.replace(self.token.expose(), "[REDACTED]");
                truncate_chars(&body, 512)
            }
            Err(ForgejoError::ResponseTooLarge) => "error response exceeds configured bound".into(),
            Err(_) => "unable to read error response".into(),
        }
    }
}

#[path = "client_capabilities.rs"]
mod capabilities;
#[path = "client_identity.rs"]
mod identity;

#[derive(Deserialize)]
struct RawRegistration {
    id: u64,
    uuid: String,
    token: String,
}

#[derive(Debug, Serialize)]
struct RegisterRunnerRequest<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    ephemeral: bool,
}
