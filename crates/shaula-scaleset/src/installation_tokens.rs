//! Installation token minting and token-scoped probes (spec 0011
//! §4.1/§5.3), split from  to keep files within the
//! 400-line budget (AGENTS.md).

use shaula_core::secret::SecretString;

use crate::body::BoundedBody;
use crate::client::{classify_response, github_request, retry_after_ms};
use crate::error::ScalesetError;
use crate::installation::{
    AppInstallationResolver, InstallationLookup, MetadataReachability, RepoIdentity,
};

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
        let response = github_request(
            self.http.get(format!(
                "{}/installation/repositories?per_page=1",
                self.github_api_base
            )),
            token,
        )
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
        let response = match github_request(
            self.http.get(format!(
                "{}/repos/{owner}/{repository}",
                self.github_api_base
            )),
            token,
        )
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
        let body = match response.bounded_json::<serde_json::Value>().await {
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
            Ok(response) => response,
            Err(ScalesetError::Status { status: 404, .. }) => {
                return InstallationLookup::NotFound;
            }
            Err(ScalesetError::RateLimited {
                retry_after_secs, ..
            }) => {
                return InstallationLookup::Transient {
                    retry_after_ms: retry_after_ms(retry_after_secs),
                };
            }
            Err(_) => {
                return InstallationLookup::Transient {
                    retry_after_ms: None,
                };
            }
        };
        Self::classify_installation_response(
            response,
            app_id,
            Some(installation_id),
            None,
            crate::installation::required_permission_for_target(target),
        )
        .await
    }
}
