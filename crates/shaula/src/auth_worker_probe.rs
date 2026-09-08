//! Real runner access probes for v2 validation (spec 0011 §4.1 step 5),
//! split from `auth_worker_v2` to keep files within the 400-line budget.
//! A probe is a READ through the resolved installation credential — a
//! successful client construction is never treated as access. Rate
//! limiting maps onto the bounded-retry outcome, never a permission
//! verdict (spec 0011 §5.3).

use shaula_core::error::CoreResult;
use shaula_core::github::GitHubTarget;
use shaula_core::ports::Clock;
use shaula_core::secret::SecretString;
use shaula_scaleset::{Credential, ScalesetClient};
use std::sync::Arc;

/// Transport/configuration seam for validator clients (R1): production
/// wiring passes the fixed `github.com` API base with test mode off;
/// scripted tests pin a local server so the REAL flow is exercised.
#[derive(Debug, Clone)]
pub(super) struct WorkerEndpoints {
    pub api_base: String,
    /// Test-only: construct probe clients against the scripted local
    /// server (`api_base`). Never set by production wiring.
    pub allow_test_endpoints: bool,
}

impl WorkerEndpoints {
    /// Production endpoints: the fixed `github.com` configuration.
    pub fn production() -> Self {
        Self {
            api_base: shaula_scaleset::config::GITHUB_COM_API_BASE.to_string(),
            allow_test_endpoints: false,
        }
    }

    pub(crate) fn probe_client(
        &self,
        target: GitHubTarget,
        credential: Credential,
        clock: Arc<dyn Clock>,
    ) -> CoreResult<ScalesetClient> {
        #[cfg(debug_assertions)]
        if self.allow_test_endpoints {
            let http = shaula_scaleset::client::production_http_client().map_err(|e| {
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

/// Outcome of one probe step in the validation flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeOutcome {
    Proven,
    /// Terminal Candidate rejection with a stable reason.
    Terminal(&'static str),
    /// Transient: the Candidate stays Validating with a bounded retry,
    /// honoring any deadline GitHub supplied.
    Retry {
        retry_after_ms: Option<i64>,
    },
}

/// The real runner access probe for one concrete target: a read through
/// the resolved installation credential.
pub(super) async fn probe_runner_access(
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
        Err(shaula_scaleset::ScalesetError::RateLimited {
            retry_after_secs, ..
        }) => ProbeOutcome::Retry {
            retry_after_ms: retry_after_secs.map(|s| s.max(0).saturating_mul(1000)),
        },
        Err(shaula_scaleset::ScalesetError::Status {
            status: 401 | 403, ..
        }) => ProbeOutcome::Terminal("PermissionDenied"),
        Err(shaula_scaleset::ScalesetError::Status { status: 404, .. }) => {
            ProbeOutcome::Terminal("TargetHiddenOrNotFound")
        }
        Err(_) => ProbeOutcome::Retry {
            retry_after_ms: None,
        },
    }
}
