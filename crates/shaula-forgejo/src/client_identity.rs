//! Scope names and user-relative routes are pinned to immutable numeric identity.

use super::{ForgejoClient, ForgejoError, ForgejoScope};

impl ForgejoClient {
    pub fn with_expected_scope_identity(mut self, id: u64) -> Result<Self, ForgejoError> {
        if id == 0 {
            return Err(ForgejoError::Configuration(
                "scope identity must be positive".into(),
            ));
        }
        self.expected_scope_id = Some(id);
        Ok(self)
    }

    pub(crate) async fn scope_identity(&self) -> Result<Option<u64>, ForgejoError> {
        let path = match &self.scope {
            ForgejoScope::Instance => return Ok(None),
            ForgejoScope::User => "api/v1/user".to_string(),
            _ => self
                .scope
                .api_path()
                .trim_start_matches('/')
                .trim_end_matches("/actions/runners")
                .to_string(),
        };
        #[derive(serde::Deserialize)]
        struct Identity {
            id: u64,
        }
        let endpoint = self.base_url.join(&path)?;
        let response = self
            .authorized(reqwest::Method::GET, endpoint)
            .send()
            .await
            .map_err(|error| ForgejoError::unavailable(error.to_string()))?;
        let identity: Identity = self.decode(response).await?;
        if identity.id == 0 {
            return Err(ForgejoError::InvalidResponse(
                "scope identity missing".into(),
            ));
        }
        Ok(Some(identity.id))
    }

    pub(super) async fn verify_scope_identity(&self) -> Result<(), ForgejoError> {
        if let Some(expected) = self.expected_scope_id {
            if self.scope_identity().await? != Some(expected) {
                return Err(ForgejoError::PermissionDenied);
            }
        }
        Ok(())
    }
}
