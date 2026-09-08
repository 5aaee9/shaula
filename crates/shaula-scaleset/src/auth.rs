//! Credential chain: GitHub App JWT → installation token, or PAT →
//! registration token → Actions Service admin connection (URL + JWT).
//! Secrets stay inside this module; derived tokens never leak through
//! `Debug` or errors.

use std::sync::Arc;

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use shaula_core::ports::Clock;
use shaula_core::secret::SecretString;
use tokio::sync::Mutex;
use wire::{ActionsServiceAdminConnection, InstallationAccessToken, RegistrationToken};

use crate::error::ScalesetError;
use crate::wire;
#[path = "auth_repository.rs"]
mod repository;
#[path = "auth_validation.rs"]
mod validation;

#[derive(Serialize)]
struct AppJwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// Produces a GitHub App JWT (RS256) from a PEM private key.
/// Mirrors the Go client: issued 60s in the past, expires 9 minutes later.
pub fn app_jwt(
    client_id: &str,
    private_key_pem: &str,
    now_unix_seconds: i64,
) -> Result<String, ScalesetError> {
    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes()).map_err(|_| {
        ScalesetError::Configuration {
            summary: "github app private key is not valid PEM".into(),
        }
    })?;
    let claims = AppJwtClaims {
        iat: now_unix_seconds - 60,
        exp: now_unix_seconds + 9 * 60,
        iss: client_id.to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(|e| {
        ScalesetError::Configuration {
            summary: format!("failed to sign app jwt: {e}"),
        }
    })
}

/// Non-secret admin connection facts kept for refresh decisions.
#[derive(Clone)]
pub struct AdminConnection {
    pub actions_service_url: String,
    pub admin_token: String,
    /// Unix seconds at which the admin JWT expires.
    pub expires_at_unix: i64,
}

/// Credential kind handed to the bootstrap chain.
#[derive(Clone)]
pub enum Credential {
    Pat(SecretString),
    GitHubApp {
        client_id: String,
        installation_id: i64,
        private_key: SecretString,
    },
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credential::Pat(_) => f.write_str("Credential::Pat(REDACTED)"),
            Credential::GitHubApp {
                client_id,
                installation_id,
                ..
            } => f
                .debug_struct("Credential::GitHubApp")
                .field("client_id", client_id)
                .field("installation_id", installation_id)
                .finish_non_exhaustive(),
        }
    }
}

/// Manages the registration-token → admin-connection chain with
/// expiry-based refresh. One instance per authenticated target/context.
pub struct AdminTokenManager {
    credential: Credential,
    github_api_base: String,
    config_url: String,
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    state: Mutex<Option<AdminConnection>>,
    repository_id: std::sync::atomic::AtomicI64,
}

/// Minimum remaining lifetime before a proactive refresh (matches the Go
/// client's 60-second window).
const REFRESH_MARGIN: i64 = 60;

impl AdminTokenManager {
    pub(crate) fn credential_kind(&self) -> shaula_core::auth::AuthKind {
        match &self.credential {
            Credential::Pat(_) => shaula_core::auth::AuthKind::Pat,
            Credential::GitHubApp { .. } => shaula_core::auth::AuthKind::GithubApp,
        }
    }

    pub fn new(
        credential: Credential,
        github_api_base: String,
        config_url: String,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            credential,
            github_api_base,
            config_url,
            http,
            clock,
            state: Mutex::new(None),
            repository_id: std::sync::atomic::AtomicI64::new(0),
        }
    }

    /// Returns a valid admin connection, refreshing when near expiry.
    pub async fn connection(&self) -> Result<AdminConnection, ScalesetError> {
        let mut guard = self.state.lock().await;
        if let Some(existing) = guard.as_ref() {
            let now = self.clock.now_unix_ms() / 1000;
            if existing.expires_at_unix - now > REFRESH_MARGIN {
                return Ok(existing.clone());
            }
        }
        let refreshed = self.bootstrap().await?;
        *guard = Some(refreshed.clone());
        Ok(refreshed)
    }

    /// A fresh GitHub REST installation token (metadata reads such as
    /// `GET /repos/{owner}/{repo}` for the target identity proof).
    pub(crate) async fn installation_token(&self) -> Result<String, ScalesetError> {
        match &self.credential {
            Credential::Pat(pat) => Ok(pat.expose().to_string()),
            Credential::GitHubApp {
                client_id,
                installation_id,
                private_key,
            } => {
                let now = self.clock.now_unix_ms() / 1000;
                let jwt = app_jwt(client_id, private_key.expose(), now)?;
                let token = self
                    .fetch_installation_token(
                        &jwt,
                        *installation_id,
                        &serde_json::json!({"permissions": {"metadata": "read"}}),
                    )
                    .await?;
                Ok(token.token)
            }
        }
    }

    /// Read access to the exact credential this manager authenticates.
    pub(crate) fn credential(&self) -> &Credential {
        &self.credential
    }

    /// The REST API base for authenticated metadata reads.
    pub(crate) fn api_base(&self) -> &str {
        &self.github_api_base
    }

    /// Forces a full refresh (used on 401 from the Actions Service).
    pub async fn force_refresh(&self) -> Result<AdminConnection, ScalesetError> {
        let mut guard = self.state.lock().await;
        let refreshed = self.bootstrap().await?;
        *guard = Some(refreshed.clone());
        Ok(refreshed)
    }

    async fn bootstrap(&self) -> Result<AdminConnection, ScalesetError> {
        let registration = self.fetch_registration_token().await?;
        let conn = self.fetch_admin_connection(&registration.token).await?;
        let expires_at_unix =
            parse_jwt_exp(&conn.admin_token).ok_or(ScalesetError::MalformedResponse {
                summary: "admin token lacks exp claim".into(),
            })?;
        Ok(AdminConnection {
            actions_service_url: conn.actions_service_url.trim_end_matches('/').to_string(),
            admin_token: conn.admin_token,
            expires_at_unix,
        })
    }

    async fn fetch_registration_token(&self) -> Result<RegistrationToken, ScalesetError> {
        let path = registration_token_path(&self.config_url).ok_or_else(|| {
            ScalesetError::Configuration {
                summary: "config url has no known registration path".into(),
            }
        })?;
        let url = format!("{}{path}", self.github_api_base.trim_end_matches('/'));

        let bearer = match &self.credential {
            Credential::Pat(pat) => format!("Bearer {}", pat.expose()),
            Credential::GitHubApp {
                client_id,
                private_key,
                ..
            } => {
                let now = self.clock.now_unix_ms() / 1000;
                let jwt = app_jwt(client_id, private_key.expose(), now)?;
                let installation_id = match &self.credential {
                    Credential::GitHubApp {
                        installation_id, ..
                    } => *installation_id,
                    _ => unreachable!("guarded by match above"),
                };
                let scope = self.runner_token_scope().await?;
                let token = self
                    .fetch_installation_token(&jwt, installation_id, &scope)
                    .await?;
                format!("Bearer {}", token.token)
            }
        };

        let response = self
            .http
            .post(&url)
            .header("Authorization", bearer)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(transport_error)?;
        // Rate-limited bootstrap requests classify as bounded retries
        // (headers inspected before the body) instead of terminal 403s.
        let response =
            crate::client::classify_response(response, "registration token request failed")?;
        if response.status().as_u16() != 201 {
            return Err(ScalesetError::Status {
                status: response.status().as_u16(),
                summary: "registration token request failed".into(),
            });
        }
        response
            .json::<RegistrationToken>()
            .await
            .map_err(|e| ScalesetError::MalformedResponse {
                summary: format!("registration token body invalid: {e}"),
            })
    }

    async fn fetch_admin_connection(
        &self,
        registration_token: &str,
    ) -> Result<ActionsServiceAdminConnection, ScalesetError> {
        let url = format!(
            "{}/actions/runner-registration",
            self.github_api_base.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "url": self.config_url,
            "runner_event": "register",
        });
        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("RemoteAuth {registration_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(transport_error)?;
        let response =
            crate::client::classify_response(response, "runner-registration request failed")?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ScalesetError::Status {
                status,
                summary: "runner-registration request failed".into(),
            });
        }
        let conn = response
            .json::<ActionsServiceAdminConnection>()
            .await
            .map_err(|e| ScalesetError::MalformedResponse {
                summary: format!("runner-registration body invalid: {e}"),
            })?;
        if conn.actions_service_url.is_empty() || conn.admin_token.is_empty() {
            return Err(ScalesetError::MalformedResponse {
                summary: "runner-registration response missing url or token".into(),
            });
        }
        Ok(conn)
    }
}

fn transport_error(e: reqwest::Error) -> ScalesetError {
    if e.is_timeout() || e.is_connect() {
        ScalesetError::RequestUncertain {
            summary: format!("transport failure: {e}"),
        }
    } else {
        ScalesetError::RequestUncertain {
            summary: format!("request failed: {e}"),
        }
    }
}

pub(crate) fn transport_error_pub(e: reqwest::Error) -> ScalesetError {
    transport_error(e)
}

fn registration_token_path(config_url: &str) -> Option<String> {
    let rest = config_url.strip_prefix("https://github.com/")?;
    let mut parts = rest.trim_matches('/').split('/');
    let owner = parts.next()?;
    match parts.next() {
        None => Some(format!("/orgs/{owner}/actions/runners/registration-token")),
        Some(repository) => Some(format!(
            "/repos/{owner}/{repository}/actions/runners/registration-token"
        )),
    }
}

/// Extracts the `exp` claim from an unverified JWT. Never validates
/// signatures here — the token came over TLS from the registration flow.
pub fn parse_jwt_exp(jwt: &str) -> Option<i64> {
    let payload = jwt.split('.').nth(1)?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    #[derive(serde::Deserialize)]
    struct ExpOnly {
        exp: i64,
    }
    serde_json::from_slice::<ExpOnly>(&bytes)
        .ok()
        .map(|e| e.exp)
}

/// R10-11: validates the Actions Service URL returned by the registration
/// response — the bearer-bearing requests go exactly there, so a hostile
/// value (wrong scheme, userinfo, missing host) must be refused, never
#[path = "auth_url_validation.rs"]
pub(crate) mod auth_url_validation;

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
