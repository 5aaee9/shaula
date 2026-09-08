//! Installation token minting and token-scoped probes (spec 0011
//! §4.1/§5.3), split from  to keep files within the
//! 400-line budget (AGENTS.md).

use shaula_core::auth_context::RepositorySelection;
use shaula_core::auth_policy::AccountKind;
use shaula_core::secret::SecretString;

use crate::client::{classify_response, DEFAULT_REQUEST_TIMEOUT, USER_AGENT};
use crate::error::ScalesetError;
use crate::installation::{
    AppInstallationResolver, InstallationLookup, InstallationProof, InstallationResponse,
    MetadataReachability, RepoIdentity,
};

/// G2: GitHub's Retry-After survives every helper boundary — a
/// rate-limited probe carries the deadline instead of collapsing into
/// the default backoff.
fn retry_after_ms(retry_after_secs: Option<i64>) -> Option<i64> {
    retry_after_secs.map(|s| s.max(0).saturating_mul(1000))
}

impl AppInstallationResolver {
    /// Bounded metadata probe of the installation's accessible
    /// repositories: mints a SHORT-LIVED installation access token for
    /// THIS installation and calls the installation-token endpoint
    /// `GET /installation/repositories` (page 1, per_page=1). An account
    /// with ZERO repositories still proves the route; the result is never
    /// an authorization source (spec 0011 §3).
    pub async fn installation_metadata_reachable(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
    ) -> MetadataReachability {
        let token = match self
            .mint_installation_token(app_id, private_key, installation_id)
            .await
        {
            Ok(token) => token,
            // The installation itself is gone: minting its token 404s.
            Err(ScalesetError::Status { status: 404, .. }) => {
                return MetadataReachability::NotFound
            }
            Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            }) => {
                return MetadataReachability::Transient {
                    retry_after_ms: retry_after_ms(retry_after_secs),
                }
            }
            Err(_) => {
                return MetadataReachability::Transient {
                    retry_after_ms: None,
                }
            }
        };
        let response = self
            .http
            .get(format!(
                "{}/installation/repositories?per_page=1",
                self.github_api_base
            ))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", USER_AGENT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                return MetadataReachability::Transient {
                    retry_after_ms: None,
                }
            }
        };
        match crate::client::classify_response(response, "metadata probe failed") {
            Ok(_) => MetadataReachability::Reachable,
            Err(ScalesetError::Status { status: 404, .. }) => MetadataReachability::NotFound,
            Err(ScalesetError::Status { status: 403, .. }) => {
                MetadataReachability::PermissionDenied
            }
            Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            }) => MetadataReachability::Transient {
                retry_after_ms: retry_after_ms(retry_after_secs),
            },
            _ => MetadataReachability::Transient {
                retry_after_ms: None,
            },
        }
    }

    /// Verifies one exact repository through the installation credential
    /// and returns its durable numeric identity
    /// (`GET /repos/{owner}/{repo}`: repository id + owner id), the
    /// continuity authority for unchanged exact-repository selectors
    /// (spec 0011 §4.1 step 4, §5.1).
    pub async fn repository_identity(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
        owner: &str,
        repository: &str,
    ) -> RepoIdentity {
        let token = match self
            .mint_installation_token(app_id, private_key, installation_id)
            .await
        {
            Ok(token) => token,
            Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            }) => {
                return RepoIdentity::Transient {
                    retry_after_ms: retry_after_ms(retry_after_secs),
                }
            }
            Err(_) => {
                return RepoIdentity::Transient {
                    retry_after_ms: None,
                }
            }
        };
        let response = match self
            .http
            .get(format!(
                "{}/repos/{owner}/{repository}",
                self.github_api_base
            ))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", USER_AGENT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => {
                return RepoIdentity::Transient {
                    retry_after_ms: None,
                }
            }
        };
        let response = match classify_response(response, "repository lookup failed") {
            Ok(response) => response,
            Err(ScalesetError::Status { status: 404, .. }) => return RepoIdentity::Missing,
            Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            }) => {
                return RepoIdentity::Transient {
                    retry_after_ms: retry_after_ms(retry_after_secs),
                }
            }
            Err(_) => {
                return RepoIdentity::Transient {
                    retry_after_ms: None,
                }
            }
        };
        let body = match response.json::<serde_json::Value>().await {
            Ok(body) => body,
            Err(_) => {
                return RepoIdentity::Transient {
                    retry_after_ms: None,
                }
            }
        };
        let id = body.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let owner_id = body
            .pointer("/owner/id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if !crate::route_identity::valid_id(id) || !crate::route_identity::valid_id(owner_id) {
            return RepoIdentity::Transient {
                retry_after_ms: None,
            };
        }
        RepoIdentity::Proven(id, owner_id)
    }

    /// Re-proves ONE installation by id (`GET /app/installations/{id}`, App
    /// JWT): the runtime freshness re-check that the BOUND installation
    /// still exists, still answers for THIS App and is not suspended
    /// (spec 0011 §5.3).
    pub async fn installation_by_id(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
        target: &shaula_core::github::GitHubTarget,
    ) -> InstallationLookup {
        let path = format!("/app/installations/{installation_id}");
        let response = match self.app_request(app_id, private_key, &path).await {
            Ok(Ok(response)) => response,
            Ok(Err(ScalesetError::Status { status: 404, .. })) => {
                return InstallationLookup::NotFound
            }
            Ok(Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            })) => {
                return InstallationLookup::Transient {
                    retry_after_ms: retry_after_ms(retry_after_secs),
                }
            }
            Ok(Err(_)) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                }
            }
            Err(_) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                }
            }
        };
        let parsed: InstallationResponse = match response.json().await {
            Ok(parsed) => parsed,
            Err(_) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                }
            }
        };
        if !crate::route_identity::valid_id(parsed.id)
            || !crate::route_identity::valid_id(parsed.app_id)
            || !crate::route_identity::valid_id(parsed.account.id)
        {
            return InstallationLookup::Transient {
                retry_after_ms: None,
            };
        }
        if parsed.id != installation_id {
            return InstallationLookup::IdentityMismatch;
        }
        // F3: a client-ID issuer representation is NOT comparable with
        // the numeric response id — the JWT signature plus the
        // installation id anchor the identity; a NUMERIC declaration is
        // compared exactly.
        if app_id.bytes().all(|b| b.is_ascii_digit()) && parsed.app_id.to_string() != app_id {
            return InstallationLookup::IdentityMismatch;
        }
        if parsed.suspended_at.is_some() {
            return InstallationLookup::Suspended;
        }
        let (required_name, required_access) =
            crate::installation::required_permission_for_target(target);
        let has_required_permission = parsed
            .permissions
            .get(required_name)
            .is_some_and(|granted| granted == required_access || granted == "admin");
        if !has_required_permission {
            return InstallationLookup::PermissionDenied;
        }
        InstallationLookup::Proven(InstallationProof {
            installation_id: parsed.id,
            app_id: parsed.app_id,
            account_id: parsed.account.id,
            account_kind: match parsed.account.account_type.as_str() {
                "User" => AccountKind::User,
                "Organization" => AccountKind::Organization,
                _ => return InstallationLookup::IdentityMismatch,
            },
            login: parsed.account.login,
            repository_selection: match parsed.repository_selection.as_deref() {
                Some("selected") => RepositorySelection::Selected,
                _ => RepositorySelection::All,
            },
            has_required_permission: true,
        })
    }
}
