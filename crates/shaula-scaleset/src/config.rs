//! GitHub config URL handling. Production targets resolve only to
//! `github.com` organization/repository forms; arbitrary caller URLs are
//! never fetched. Test-only endpoint injection cannot be enabled in release
//! builds (`allow_test_endpoints` stays private to this crate and is only
//! settable from tests).

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
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

    pub fn registration_token_path(&self) -> String {
        match &self.target {
            GitHubTarget::Organization { owner } => {
                format!("/orgs/{owner}/actions/runners/registration-token")
            }
            GitHubTarget::Repository { owner, repository } => {
                format!("/repos/{owner}/{repository}/actions/runners/registration-token")
            }
        }
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

/// Validates a caller-supplied config URL shape against the typed target.
/// The URL itself is never used as an endpoint.
pub fn validate_config_url_matches(url: &str, target: &GitHubTarget) -> CoreResult<()> {
    let expected = target.config_url();
    let normalized = url.trim_end_matches('/');
    if normalized.eq_ignore_ascii_case(expected.trim_end_matches('/')) {
        Ok(())
    } else {
        Err(CoreError::new(
            ReasonCode::SpecInvalid,
            "config URL does not match the typed GitHub target",
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn registration_token_paths_by_scope() {
        let org = GitHubConfig::production(GitHubTarget::organization("example-org").unwrap());
        assert_eq!(
            org.registration_token_path(),
            "/orgs/example-org/actions/runners/registration-token"
        );

        let repo = GitHubConfig::production(
            GitHubTarget::new_repository("example-org", "example-repo").unwrap(),
        );
        assert_eq!(
            repo.registration_token_path(),
            "/repos/example-org/example-repo/actions/runners/registration-token"
        );
    }

    #[test]
    fn production_uses_api_github_com() {
        let cfg = GitHubConfig::production(GitHubTarget::organization("o").unwrap());
        assert_eq!(cfg.github_api_base, "https://api.github.com");
        assert!(!cfg.allow_test_endpoints);
    }

    #[test]
    fn config_url_validation() {
        let target = GitHubTarget::organization("example-org").unwrap();
        assert!(validate_config_url_matches("https://github.com/example-org", &target).is_ok());
        assert!(validate_config_url_matches("https://github.com/example-org/", &target).is_ok());
        assert!(
            validate_config_url_matches("https://evil.example.com/example-org", &target).is_err()
        );
        assert!(validate_config_url_matches("https://github.com/other-org", &target).is_err());
    }
}
