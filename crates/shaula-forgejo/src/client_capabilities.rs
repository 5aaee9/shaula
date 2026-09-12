//! Bounded minimum-version and scoped authentication evidence.

use super::{ForgejoClient, ForgejoError, ForgejoScope};
use shaula_core::ports::forgejo::ForgejoAuthProbe;

impl ForgejoClient {
    pub async fn server_version(&self) -> Result<String, ForgejoError> {
        #[derive(serde::Deserialize)]
        struct Version {
            version: String,
        }
        let response = self
            .authorized(reqwest::Method::GET, self.base_url.join("api/v1/version")?)
            .send()
            .await
            .map_err(|_| ForgejoError::unavailable("server version read failed"))?;
        let version: Version = self.decode(response).await?;
        if !shaula_core::forgejo::supports_server_version(&version.version) {
            return Err(ForgejoError::UnsupportedServerVersion);
        }
        Ok(version.version)
    }

    /// Does not mutate the server. Version and numeric scope identity are
    /// captured together with a successful bounded inventory read.
    pub async fn probe_authentication(&self) -> Result<ForgejoAuthProbe, ForgejoError> {
        let server_version = self.server_version().await?;
        let identity = self.scope_identity().await?;
        let (principal_id, target_id) = if self.scope() == &ForgejoScope::User {
            (identity, None)
        } else {
            (None, identity)
        };
        let runner_count = self.list_runners().await?.len();
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ForgejoError::unavailable("system clock is before Unix epoch"))?;
        let checked_at_unix_ms = i64::try_from(elapsed.as_millis())
            .map_err(|_| ForgejoError::unavailable("system clock exceeds millisecond range"))?;
        Ok(ForgejoAuthProbe {
            server_version,
            principal_id,
            target_id,
            checked_at_unix_ms,
            valid_until_unix_ms: checked_at_unix_ms.saturating_add(60_000),
            runner_count: u64::try_from(runner_count).unwrap_or(u64::MAX),
        })
    }
}

#[cfg(test)]
#[path = "client_capabilities_tests.rs"]
mod tests;
