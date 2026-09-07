//! Read-only principal verification before an Auth Candidate can activate.
use super::{app_jwt, AdminTokenManager, Credential};
use crate::error::ScalesetError;
use shaula_core::registry::AuthRevisionRow;

impl AdminTokenManager {
    pub(crate) async fn validate_identity(
        &self,
        expected: &AuthRevisionRow,
    ) -> Result<(), ScalesetError> {
        let matches = match &self.credential {
            Credential::Pat(token) => {
                let user = self.identity_request("/user", token.expose()).await?;
                expected.pat_principal.as_deref().is_some_and(|principal| {
                    user.get("login")
                        .and_then(|v| v.as_str())
                        .is_some_and(|login| login.eq_ignore_ascii_case(principal))
                        && user
                            .get("id")
                            .and_then(|v| v.as_i64())
                            .is_some_and(|id| id > 0)
                })
            }
            Credential::GitHubApp {
                client_id,
                installation_id,
                private_key,
            } => {
                let jwt = app_jwt(
                    client_id,
                    private_key.expose(),
                    self.clock.now_unix_ms() / 1000,
                )?;
                let app = self.identity_request("/app", &jwt).await?;
                let install = self
                    .identity_request(&format!("/app/installations/{installation_id}"), &jwt)
                    .await?;
                let app_id = app.get("id").and_then(|v| v.as_i64());
                app_id.is_some_and(|id| {
                    id > 0
                        && (id.to_string() == *client_id
                            || app.get("client_id").and_then(|v| v.as_str()) == Some(client_id))
                }) && install.get("app_id").and_then(|v| v.as_i64()) == app_id
                    && install.get("id").and_then(|v| v.as_i64()) == Some(*installation_id)
                    && expected.installation_id == Some(*installation_id)
                    && expected.app_id.as_ref() == Some(client_id)
            }
        };
        if matches {
            Ok(())
        } else {
            Err(ScalesetError::Configuration {
                summary: "authenticated principal differs from declared identity".into(),
            })
        }
    }

    async fn identity_request(
        &self,
        path: &str,
        token: &str,
    ) -> Result<serde_json::Value, ScalesetError> {
        let response = self
            .http
            .get(format!(
                "{}{path}",
                self.github_api_base.trim_end_matches('/')
            ))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .timeout(crate::client::DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(super::transport_error_pub)?;
        if !response.status().is_success() {
            return Err(ScalesetError::Status {
                status: response.status().as_u16(),
                summary: "identity verification failed".into(),
            });
        }
        response
            .json()
            .await
            .map_err(|_| ScalesetError::MalformedResponse {
                summary: "identity response invalid".into(),
            })
    }
}
