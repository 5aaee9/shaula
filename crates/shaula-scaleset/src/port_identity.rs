//! Resolve complete numeric target identity; remote metadata is evidence, never a pin.

use shaula_core::github::GitHubTarget;
use shaula_core::ports::{AccessFailure, TargetIdentity};

use super::ScalesetClient;
use crate::client::classify_response;
use crate::route_identity::invalid_identity;

impl ScalesetClient {
    pub(crate) async fn resolve_target_identity_impl(
        &self,
        target: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure> {
        self.fetch_target_identity(target)
            .await
            .inspect_err(|failure| {
                self.invalidate_route_proof(&crate::error::ScalesetError::Authorization {
                    failure: failure.clone(),
                });
            })
    }

    async fn fetch_target_identity(
        &self,
        target: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure> {
        let path = match target {
            GitHubTarget::Organization { owner } => format!("/orgs/{owner}"),
            GitHubTarget::Repository { owner, repository } => {
                format!("/repos/{owner}/{repository}")
            }
        };
        let token = self
            .admin
            .installation_token()
            .await
            .inspect_err(|e| self.invalidate_route_proof(e))
            .map_err(|e| e.to_access_failure())?;
        let response = self
            .http
            .get(format!("{}{path}", self.admin.api_base()))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .timeout(crate::client::DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(crate::auth::transport_error_pub)
            .map_err(|e| e.to_access_failure())?;
        self.observe_response(&response);
        let response = classify_response(response, "target identity lookup failed")
            .map_err(|e| e.to_access_failure())?;
        let body: serde_json::Value = response.json().await.map_err(|_| invalid_identity())?;
        let identity = crate::route_identity::parse_target_identity(target, &body)?;
        if target != &self.config.target {
            return Err(AccessFailure::PermissionDenied);
        }
        if let Some(id) = identity.repository_id {
            self.admin
                .pin_repository_id(id)
                .map_err(|e| e.to_access_failure())?;
        }
        Ok(identity)
    }
}
