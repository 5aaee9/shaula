//! GitHub config URL handling. Production targets resolve only to
//! `github.com` organization/repository forms; arbitrary caller URLs are
//! never fetched. Test-only endpoint injection cannot be enabled in release
//! builds (`allow_test_endpoints` stays private to this crate and is only
//! settable from tests).

use shaula_core::github::GitHubTarget;

/// The Actions Service base URL for github.com targets. In production this
/// is discovered at runtime via `/actions/runner-registration`; tests may
/// pin a local URL through the registry below.
pub const GITHUB_COM_API_BASE: &str = "https://api.github.com";

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    pub target: GitHubTarget,
    /// GitHub REST API base, e.g. `https://api.github.com`.
    pub github_api_base: String,
    /// Test-only override flag; never true in release configuration.
    pub allow_test_endpoints: bool,
}

impl GitHubConfig {
    /// Production constructor: strictly `github.com` targets.
    pub fn production(target: GitHubTarget) -> Self {
        Self {
            target,
            github_api_base: GITHUB_COM_API_BASE.to_string(),
            allow_test_endpoints: false,
        }
    }

    pub fn config_url(&self) -> String {
        self.target.config_url()
    }

    #[cfg(test)]
    pub fn test_local(
        target: GitHubTarget,
        api_base: String,
        actions_service_base: String,
    ) -> (Self, String) {
        (
            Self {
                target,
                github_api_base: api_base,
                allow_test_endpoints: true,
            },
            actions_service_base,
        )
    }
}
