//! The reqwest-based Scale Set client: construction, admin-token plumbing
//! and Actions Service request mechanics.

use std::sync::Arc;
use std::time::Duration;

use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;

use crate::auth::AdminTokenManager;
use crate::auth::Credential;
use crate::config::GitHubConfig;
use crate::error::ScalesetError;
use crate::wire::API_VERSION;

/// Bounded User-Agent identifying the Shaula implementation.
pub const USER_AGENT: &str = r#"{"system":"shaula","version":"0.1.0","subsystem":"scaleset"}"#;

/// Request timeout for every outbound call; long polling is handled with a
/// longer, explicit timeout at the poll call site.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// THE production HTTP client for GitHub REST/App traffic: the fixed
/// Shaula User-Agent (GitHub refuses requests without one), a bounded
/// timeout and redirects DISABLED so credential-bearing requests can never
/// be followed to another origin. Every caller of GitHub HTTP uses this.
pub fn production_http_client() -> Result<reqwest::Client, ScalesetError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(DEFAULT_REQUEST_TIMEOUT)
        // Credential-bearing calls never follow redirects.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ScalesetError::Configuration {
            summary: format!("http client build failed: {e}"),
        })
}

/// Classifies one completed GitHub response: success passes through,
/// rate-limited `403`/`429` map onto the bounded-retry variant (headers
/// inspected BEFORE the body is consumed), everything else stays a plain
/// status failure.
pub(crate) fn classify_response(
    response: reqwest::Response,
    summary: &str,
) -> Result<reqwest::Response, ScalesetError> {
    let status = response.status().as_u16();
    if response.status().is_success() {
        return Ok(response);
    }
    if let Some(error) = rate_limit_error(&response, summary) {
        return Err(error);
    }
    Err(ScalesetError::Status {
        status,
        summary: summary.to_string(),
    })
}

pub(crate) fn rate_limit_error(
    response: &reqwest::Response,
    summary: &str,
) -> Option<ScalesetError> {
    let status = response.status().as_u16();
    let headers = response.headers();
    let primary = headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "0");
    let retry_after = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    if status == 429 || (status == 403 && (primary || retry_after.is_some())) {
        return Some(ScalesetError::RateLimited {
            retry_after_secs: retry_after,
            summary: summary.to_string(),
        });
    }
    None
}

pub struct ScalesetClient {
    pub(crate) http: reqwest::Client,
    pub(crate) admin: AdminTokenManager,
    pub(crate) config: GitHubConfig,
    pub(crate) clock: Arc<dyn Clock>,
    /// Route-proof freshness state (spec 0011 §5.3), owned by THIS exact
    /// target+context client: per-route scope, never global.
    pub(crate) proof: tokio::sync::Mutex<crate::route_proof::RouteProofState>,
    /// The PERSISTED exact Resolved Auth Context this client must match
    /// (F4): refreshed route identity is compared against it, so a
    /// drifting repository/installation can never mint a fresh proof.
    pub(crate) expected_context: Option<shaula_core::auth_context::ResolvedAuthContext>,
    /// G6: invalidation generation — bumped WITHOUT the refresh mutex so
    /// an observed failure during another task's refresh is never
    /// dropped; a refresh started before the bump refuses to publish.
    pub(crate) proof_epoch: std::sync::atomic::AtomicU64,
}

impl ScalesetClient {
    /// The actual runner access probe (spec 0011 §4.1 step 5): a
    /// read-only runner-group listing through the installation credential.
    /// A successful client construction is NEVER treated as access proof.
    /// Rate-limited `403`/`429` map onto the bounded-retry variant, so a
    /// throttled probe is never misread as a permission denial.
    pub async fn probe_actions_access(&self) -> Result<(), ScalesetError> {
        let response = self
            .actions_service_request(
                reqwest::Method::GET,
                "_apis/runtime/runnergroups/",
                &[],
                None,
                DEFAULT_REQUEST_TIMEOUT,
            )
            .await?;
        classify_response(response, "target access verification failed").map(|_| ())
    }
    /// Builds a client for a production `github.com` target.
    pub fn production(
        target: GitHubTarget,
        credential: Credential,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ScalesetError> {
        let http = production_http_client()?;
        let config = GitHubConfig::production(target);
        let admin = AdminTokenManager::new(
            credential,
            config.github_api_base.clone(),
            config.config_url(),
            http.clone(),
            clock.clone(),
        );
        Ok(Self {
            http,
            admin,
            config,
            clock,
            proof: tokio::sync::Mutex::new(crate::route_proof::RouteProofState::default()),
            expected_context: None,
            proof_epoch: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Test-only construction against local servers. Hidden from docs and
    /// compiled out of release binaries, so endpoint injection can never be
    /// enabled in a release configuration.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn with_local_servers(
        target: GitHubTarget,
        credential: Credential,
        github_api_base: String,
        clock: Arc<dyn Clock>,
        http: reqwest::Client,
    ) -> Self {
        let config = GitHubConfig {
            target,
            github_api_base,
            allow_test_endpoints: true,
        };
        let admin = AdminTokenManager::new(
            credential,
            config.github_api_base.clone(),
            config.config_url(),
            http.clone(),
            clock.clone(),
        );
        Self {
            http,
            admin,
            config,
            clock,
            proof: tokio::sync::Mutex::new(crate::route_proof::RouteProofState::default()),
            expected_context: None,
            proof_epoch: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Binds the PERSISTED exact Resolved Auth Context this client must
    /// match (F4): refreshed route identity is compared against it.
    pub fn with_expected_context(
        mut self,
        context: Option<shaula_core::auth_context::ResolvedAuthContext>,
    ) -> Self {
        self.admin
            .bind_expected_repository(context.as_ref().and_then(|c| c.repository_id));
        self.expected_context = context;
        // Builder-style rebinding consumes the client but may follow a
        // previous proof. Its old cache cannot authorize a new context.
        self.proof = tokio::sync::Mutex::new(crate::route_proof::RouteProofState::default());
        self.proof_epoch
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self
    }

    /// Issues an authenticated request against the Actions Service and
    /// returns the raw response (never followed through redirects).
    pub(crate) async fn actions_service_request(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
        timeout: Duration,
    ) -> Result<reqwest::Response, ScalesetError> {
        let conn = self
            .admin
            .connection()
            .await
            .inspect_err(|e| self.invalidate_route_proof(e))?;
        let response = self
            .send_once(&conn, method.clone(), path, query, body.as_ref(), timeout)
            .await?;
        // R10-11: an expired admin JWT is retried ONCE against the original
        // request after a forced refresh — the caller no longer has to know
        // that a 401 can be transient.
        if response.status().as_u16() == 401 {
            let refreshed = self
                .admin
                .force_refresh()
                .await
                .inspect_err(|e| self.invalidate_route_proof(e))?;
            return self
                .send_once(&refreshed, method, path, query, body.as_ref(), timeout)
                .await;
        }
        Ok(response)
    }

    /// One authenticated send against a FIXED connection. `R10-11`: the
    /// service URL is validated (https, no userinfo, real host) before any
    /// bearer token is attached, so a compromised registration response
    /// can never redirect the credential.
    #[allow(clippy::too_many_arguments)]
    async fn send_once(
        &self,
        conn: &crate::auth::AdminConnection,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
        timeout: Duration,
    ) -> Result<reqwest::Response, ScalesetError> {
        let mut url = self.service_url(&conn.actions_service_url, path)?;
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())))
            .append_pair("api-version", API_VERSION);

        let bearer = format!("Bearer {}", conn.admin_token);
        let mut request = self
            .http
            .request(method, url)
            .header("Authorization", bearer)
            .header("Content-Type", "application/json")
            .timeout(timeout);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(crate::auth::transport_error_pub)?;
        self.observe_response(&response);
        Ok(response)
    }

    /// New management effects require fresh route evidence on every
    /// dispatch, including a single retry after an expired admin token.
    /// Read and historical cleanup requests keep their separate access path.
    pub(crate) async fn new_effect_request(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<reqwest::Response, ScalesetError> {
        self.authorized_effect_request(reqwest::Method::POST, path, Some(body))
            .await
    }

    pub(crate) async fn authorized_effect_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, ScalesetError> {
        let mut retried = false;
        loop {
            self.ensure_route_proof_impl()
                .await
                .map_err(|failure| ScalesetError::Authorization { failure })?;
            let conn = self
                .admin
                .connection()
                .await
                .inspect_err(|e| self.invalidate_route_proof(e))?;
            // Waiting for a shared admin refresh must not outlive proof.
            self.ensure_route_proof_impl()
                .await
                .map_err(|failure| ScalesetError::Authorization { failure })?;
            let response = self
                .send_once(
                    &conn,
                    method.clone(),
                    path,
                    &[],
                    body.as_ref(),
                    DEFAULT_REQUEST_TIMEOUT,
                )
                .await?;
            if response.status().as_u16() != 401 || retried {
                return Ok(response);
            }
            // send_once invalidated the proof; the loop re-checks identity,
            // permissions and the complete token chain before retrying.
            retried = true;
        }
    }

    /// Observe every response, including queue-token requests and a retried
    /// admin request, before a caller consumes the body or handles its status.
    pub(crate) fn observe_response(&self, response: &reqwest::Response) {
        self.invalidate_route_proof(&ScalesetError::Status {
            status: response.status().as_u16(),
            summary: "access failure".into(),
        });
    }

    pub(crate) fn service_url(
        &self,
        base: &str,
        path: &str,
    ) -> Result<reqwest::Url, ScalesetError> {
        let mut url = crate::auth::auth_url_validation::validate_actions_service_url(
            base,
            self.config.allow_test_endpoints,
        )?;
        if !path.is_empty() {
            url.path_segments_mut()
                .map_err(|_| ScalesetError::MalformedResponse {
                    summary: "invalid service path".into(),
                })?
                .pop_if_empty()
                .extend(path.trim_start_matches('/').split('/'));
        }
        Ok(url)
    }
}
