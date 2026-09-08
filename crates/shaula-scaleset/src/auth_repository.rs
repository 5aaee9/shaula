//! Numeric repository scoping for runner credentials.

use super::*;
use std::sync::atomic::Ordering;

impl AdminTokenManager {
    pub(crate) fn bind_expected_repository(&mut self, id: Option<i64>) {
        *self.repository_id.get_mut() = id.unwrap_or(0);
        // A previously cached admin token belongs to its original scope.
        *self.state.get_mut() = None;
    }

    pub(crate) fn pin_repository_id(&self, id: i64) -> Result<(), ScalesetError> {
        if !crate::route_identity::valid_id(id) {
            return Err(ScalesetError::Authorization {
                failure: crate::route_identity::invalid_identity(),
            });
        }
        match self
            .repository_id
            .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(()),
            Err(pinned) if pinned == id => Ok(()),
            Err(_) => Err(ScalesetError::Authorization {
                failure: shaula_core::ports::AccessFailure::PermissionDenied,
            }),
        }
    }

    pub(super) async fn runner_token_scope(&self) -> Result<serde_json::Value, ScalesetError> {
        let path = registration_token_path(&self.config_url).ok_or_else(|| {
            ScalesetError::Configuration {
                summary: "config url has no known registration path".into(),
            }
        })?;
        let Some(repo_path) = path
            .strip_prefix("/repos/")
            .and_then(|p| p.strip_suffix("/actions/runners/registration-token"))
        else {
            return Ok(serde_json::json!({})); // Organization runner scope.
        };
        let mut id = self.repository_id.load(Ordering::Acquire);
        if id == 0 {
            // A proof-only worker can enter the bootstrap chain before a
            // route proof exists. Discover identity using metadata-only
            // authority, then mint a separate runner token for that ID.
            let (owner, repository) =
                repo_path
                    .split_once('/')
                    .ok_or_else(|| ScalesetError::Configuration {
                        summary: "invalid repository target".into(),
                    })?;
            let target = shaula_core::github::GitHubTarget::new_repository(owner, repository)
                .map_err(|_| ScalesetError::Configuration {
                    summary: "invalid repository target".into(),
                })?;
            let token = self.installation_token().await?;
            let response = self
                .http
                .get(format!(
                    "{}/repos/{repo_path}",
                    self.github_api_base.trim_end_matches('/')
                ))
                .bearer_auth(token)
                .header("Accept", "application/vnd.github+json")
                .timeout(crate::client::DEFAULT_REQUEST_TIMEOUT)
                .send()
                .await
                .map_err(transport_error)?;
            let response =
                crate::client::classify_response(response, "repository identity lookup failed")?;
            let body: serde_json::Value =
                response
                    .json()
                    .await
                    .map_err(|_| ScalesetError::MalformedResponse {
                        summary: "repository identity response invalid".into(),
                    })?;
            id = crate::route_identity::parse_target_identity(&target, &body)
                .map_err(|failure| ScalesetError::Authorization { failure })?
                .repository_id
                .ok_or_else(|| ScalesetError::MalformedResponse {
                    summary: "repository id missing".into(),
                })?;
            self.pin_repository_id(id)?;
        }
        if !crate::route_identity::valid_id(id) {
            return Err(ScalesetError::Authorization {
                failure: crate::route_identity::invalid_identity(),
            });
        }
        Ok(serde_json::json!({"repository_ids": [id], "permissions": {"administration": "write"}}))
    }

    pub(super) async fn fetch_installation_token(
        &self,
        app_jwt: &str,
        installation_id: i64,
        body: &serde_json::Value,
    ) -> Result<InstallationAccessToken, ScalesetError> {
        let url = format!(
            "{}/app/installations/{installation_id}/access_tokens",
            self.github_api_base.trim_end_matches('/')
        );
        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        let response =
            crate::client::classify_response(response, "installation token request failed")?;
        if response.status().as_u16() != 201 {
            return Err(ScalesetError::Status {
                status: response.status().as_u16(),
                summary: "installation token request failed".into(),
            });
        }
        response
            .json::<InstallationAccessToken>()
            .await
            .map_err(|e| ScalesetError::MalformedResponse {
                summary: format!("installation token body invalid: {e}"),
            })
    }
}
