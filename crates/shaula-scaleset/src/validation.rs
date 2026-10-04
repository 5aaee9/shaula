//! Real runner access probes for v2 validation (spec 0011 §4.1 step 5),
//! split from `auth_worker_v2` to keep files within the 400-line budget.
//! A probe is a READ through the resolved installation credential — a
//! successful client construction is never treated as access. Rate
//! limiting maps onto the bounded-retry outcome, never a permission
//! verdict (spec 0011 §5.3).

use crate::{AppInstallationResolver, Credential, ScalesetClient};
use shaula_core::auth_policy::TargetSelector;
use shaula_core::auth_validation::{
    GitHubValidationPort, InstallationLookup, MetadataReachability, ProbeOutcome, RepoIdentity,
};
use shaula_core::error::CoreResult;
use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;
use shaula_core::secret::SecretString;
use std::sync::Arc;

/// Transport/configuration seam for validator clients (R1): production
/// wiring passes the fixed `github.com` API base with test mode off;
/// scripted tests pin a local server so the REAL flow is exercised.
#[derive(Debug, Clone)]
pub struct WorkerEndpoints {
    pub api_base: String,
    /// Test-only: construct probe clients against the scripted local
    /// server (`api_base`). Never set by production wiring.
    pub allow_test_endpoints: bool,
}

impl WorkerEndpoints {
    /// Production endpoints: the fixed `github.com` configuration.
    pub fn production() -> Self {
        Self {
            api_base: crate::config::GITHUB_COM_API_BASE.to_string(),
            allow_test_endpoints: false,
        }
    }

    pub fn probe_client(
        &self,
        target: GitHubTarget,
        credential: Credential,
        clock: Arc<dyn Clock>,
    ) -> CoreResult<ScalesetClient> {
        #[cfg(debug_assertions)]
        if self.allow_test_endpoints {
            let http = crate::client::production_http_client().map_err(|e| {
                shaula_core::error::CoreError::new(
                    shaula_core::error::ReasonCode::Internal,
                    e.summary(),
                )
            })?;
            return Ok(ScalesetClient::with_local_servers(
                target,
                credential,
                self.api_base.clone(),
                clock,
                http,
            ));
        }
        ScalesetClient::production(target, credential, clock).map_err(|e| {
            shaula_core::error::CoreError::new(
                shaula_core::error::ReasonCode::Internal,
                e.summary(),
            )
        })
    }
}

/// The real runner access probe for one concrete target: a read through
/// the resolved installation credential.
async fn probe_runner_access(
    target: &GitHubTarget,
    app_id: &str,
    installation_id: i64,
    private_key: &SecretString,
    clock: &Arc<dyn Clock>,
    endpoints: &WorkerEndpoints,
) -> ProbeOutcome {
    let credential = Credential::GitHubApp {
        client_id: app_id.to_string(),
        installation_id,
        private_key: private_key.clone(),
    };
    let Ok(client) = endpoints.probe_client(target.clone(), credential, clock.clone()) else {
        return ProbeOutcome::Retry {
            retry_after_ms: None,
        };
    };
    match client.probe_actions_access().await {
        Ok(()) => ProbeOutcome::Proven,
        Err(crate::ScalesetError::RateLimited {
            retry_after_secs, ..
        }) => ProbeOutcome::Retry {
            retry_after_ms: retry_after_secs.map(|s| s.max(0).saturating_mul(1000)),
        },
        Err(crate::ScalesetError::Status {
            status: 401 | 403, ..
        }) => ProbeOutcome::Terminal("PermissionDenied"),
        Err(crate::ScalesetError::Status { status: 404, .. }) => {
            ProbeOutcome::Terminal("TargetHiddenOrNotFound")
        }
        Err(_) => ProbeOutcome::Retry {
            retry_after_ms: None,
        },
    }
}

/// Adapter instance shared by one candidate validation pass.
pub struct GitHubValidation {
    resolver: AppInstallationResolver,
    endpoints: WorkerEndpoints,
    clock: Arc<dyn Clock>,
}
impl GitHubValidation {
    pub fn new(endpoints: WorkerEndpoints, clock: Arc<dyn Clock>) -> CoreResult<Self> {
        let http = crate::client::production_http_client().map_err(|e| {
            shaula_core::error::CoreError::new(
                shaula_core::error::ReasonCode::Internal,
                e.summary(),
            )
        })?;
        Ok(Self {
            resolver: AppInstallationResolver::new(endpoints.api_base.clone(), http, clock.clone()),
            endpoints,
            clock,
        })
    }
}
#[async_trait::async_trait]
impl GitHubValidationPort for GitHubValidation {
    async fn verify_app(&self, app_id: &str, private_key: &SecretString) -> ProbeOutcome {
        match self.resolver.verify_app(app_id, private_key).await {
            Ok(_) => ProbeOutcome::Proven,
            Err(crate::ScalesetError::RateLimited {
                retry_after_secs, ..
            }) => ProbeOutcome::Retry {
                retry_after_ms: retry_after_secs.map(|s| s.max(0).saturating_mul(1000)),
            },
            Err(
                crate::ScalesetError::Status {
                    status: 401 | 403 | 404,
                    ..
                }
                | crate::ScalesetError::Configuration { .. },
            ) => ProbeOutcome::Terminal("Unauthenticated"),
            Err(_) => ProbeOutcome::Retry {
                retry_after_ms: None,
            },
        }
    }
    async fn installation_for_selector(
        &self,
        app_id: &str,
        private_key: &SecretString,
        selector: &TargetSelector,
    ) -> InstallationLookup {
        self.resolver
            .installation_for_selector(app_id, private_key, selector)
            .await
    }
    async fn installation_metadata_reachable(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
    ) -> MetadataReachability {
        self.resolver
            .installation_metadata_reachable(app_id, private_key, installation_id)
            .await
    }
    async fn repository_identity(
        &self,
        app_id: &str,
        private_key: &SecretString,
        installation_id: i64,
        owner: &str,
        repository: &str,
    ) -> RepoIdentity {
        self.resolver
            .repository_identity(app_id, private_key, installation_id, owner, repository)
            .await
    }
    async fn probe_runner_access(
        &self,
        target: &GitHubTarget,
        app_id: &str,
        installation_id: i64,
        private_key: &SecretString,
    ) -> ProbeOutcome {
        probe_runner_access(
            target,
            app_id,
            installation_id,
            private_key,
            &self.clock,
            &self.endpoints,
        )
        .await
    }
}
