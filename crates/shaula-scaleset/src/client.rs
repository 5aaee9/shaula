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

pub struct ScalesetClient {
    pub(crate) http: reqwest::Client,
    pub(crate) admin: AdminTokenManager,
    pub(crate) config: GitHubConfig,
}

impl ScalesetClient {
    /// Verifies the declared principal and access to this exact configured target.
    pub async fn validate_auth(
        &self,
        expected: &shaula_core::registry::AuthRevisionRow,
    ) -> Result<(), ScalesetError> {
        self.admin.validate_identity(expected).await?;
        let response = self
            .actions_service_request(
                reqwest::Method::GET,
                "_apis/runtime/runnergroups/",
                &[],
                None,
                DEFAULT_REQUEST_TIMEOUT,
            )
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ScalesetError::Status {
                status: response.status().as_u16(),
                summary: "target access verification failed".into(),
            })
        }
    }
    /// Builds a client for a production `github.com` target.
    pub fn production(
        target: GitHubTarget,
        credential: Credential,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ScalesetError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            // Credential-bearing calls never follow redirects.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ScalesetError::Configuration {
                summary: format!("http client build failed: {e}"),
            })?;
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
        }
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
        let conn = self.admin.connection().await?;
        let response = self
            .send_once(&conn, method.clone(), path, query, body.as_ref(), timeout)
            .await?;
        // R10-11: an expired admin JWT is retried ONCE against the original
        // request after a forced refresh — the caller no longer has to know
        // that a 401 can be transient.
        if response.status().as_u16() == 401 {
            let refreshed = self.admin.force_refresh().await?;
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
        request
            .send()
            .await
            .map_err(crate::auth::transport_error_pub)
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
