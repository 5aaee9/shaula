//! Scripted GitHub + Actions Service mock for the REAL v2 worker tests
//! (R1/R2), split to keep files within the 400-line budget.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)] // test-support module: some helpers serve specific scenarios

use axum::response::IntoResponse;
use axum::{routing::get, routing::post, Json, Router};
use base64::Engine;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use shaula_core::ports::Clock;

pub(crate) struct Now;
impl Clock for Now {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

pub(crate) struct Mock {
    pub(crate) base: String,
    pub(crate) user_agents: Arc<std::sync::Mutex<Vec<String>>>,
    pub(crate) metadata_mints: Arc<AtomicUsize>,
    /// Discovery GETs for the throttled/flaky accounts (scheduling tests).
    pub(crate) discovery_reads: Arc<AtomicUsize>,
    pub(crate) repo_id: Arc<std::sync::atomic::AtomicI64>,
    pub(crate) repo_owner_id: Arc<std::sync::atomic::AtomicI64>,
    pub(crate) scale_set_creates: Arc<AtomicUsize>,
    pub(crate) installation_reads: Arc<std::sync::Mutex<Vec<i64>>>,
}

/// Scenario knobs for the scripted server.
#[derive(Clone, Default)]
pub(crate) struct MockConfig {
    /// Ordinary 403 on the Actions runner-group read.
    pub deny_runner_groups: bool,
    /// Rate-limit 403 (+Retry-After) on the installation token mint.
    pub throttle_token_mint: bool,
}

/// Scripted GitHub + Actions Service. `deny_runner_groups` makes the
/// Actions runner-group read answer an ORDINARY 403 for the org probe.
pub(crate) async fn mock_server(deny_runner_groups: bool) -> Mock {
    mock_server_cfg(MockConfig {
        deny_runner_groups,
        ..MockConfig::default()
    })
    .await
}

pub(crate) async fn mock_server_cfg(cfg: MockConfig) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let user_agents: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let metadata_mints: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let ua_for_app = user_agents.clone();
    let token_for_meta = metadata_mints.clone();
    let discovery_reads: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let flaky_reads = discovery_reads.clone();
    let slowlimit_reads = discovery_reads.clone();
    let throttle_mint = cfg.throttle_token_mint;
    let deny_runner_groups = cfg.deny_runner_groups;
    let exp = 1_800_003_600i64;
    let admin_token = format!(
        "e30.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#))
    );
    let service = format!("{base}/actions");
    let app = Router::new()
        .route(
            "/app",
            get(move |headers: axum::http::HeaderMap| {
                let ua = ua_for_app.clone();
                async move {
                    ua.lock().unwrap().push(
                        headers
                            .get("user-agent")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string(),
                    );
                    Json(serde_json::json!({"id": 4863460}))
                }
            }),
        )
        .route(
            "/orgs/Indexyz/installation",
            get(|| async { Json(crate::auth_worker_v2::tests::installation(11, 4863460, "Indexyz", "all")) }),
        )
        .route(
            "/users/5aaee9/installation",
            get(|| async { Json(crate::auth_worker_v2::tests::installation(22, 4863460, "5aaee9", "selected")) }),
        )
        .route(
            "/users/nolimit/installation",
            get(move || {
                let reads = slowlimit_reads.clone();
                async move {
                    reads.fetch_add(1, Ordering::SeqCst);
                    (
                        axum::http::StatusCode::FORBIDDEN,
                        [("x-ratelimit-remaining", "0"), ("retry-after", "120")],
                        Json(serde_json::json!({"message": "rate limited"})),
                    )
                }
            }),
        )
        .route(
            // G2: a no-header transient failure (plain 500) exercises the
            // bounded DEFAULT backoff path.
            "/users/flaky/installation",
            get(move || {
                let reads = flaky_reads.clone();
                async move {
                    if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
                    } else {
                        Json(crate::auth_worker_v2::tests::installation(22, 4863460, "flaky", "all")).into_response()
                    }
                }
            }),
        )
        .route(
            "/app/installations/11/access_tokens",
            post(move || async move {
                if throttle_mint {
                    return (
                        axum::http::StatusCode::FORBIDDEN,
                        [("x-ratelimit-remaining", "0"), ("retry-after", "45")],
                        Json(serde_json::json!({"message": "rate limited"})),
                    )
                        .into_response();
                }
                (
                    axum::http::StatusCode::CREATED,
                    Json(serde_json::json!({"token": "inst-11-token", "expires_at": "2027-01-01T00:00:00Z"})),
                )
                    .into_response()
            }),
        )
        .route(
            "/app/installations/22/access_tokens",
            post(move || {
                let counter = token_for_meta.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (
                        axum::http::StatusCode::CREATED,
                        Json(serde_json::json!({"token": "inst-22-token", "expires_at": "2027-01-01T00:00:00Z"})),
                    )
                }
            }),
        )
        .route(
            "/installation/repositories",
            get(|headers: axum::http::HeaderMap| async move {
                use axum::response::IntoResponse;
                let token = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .trim_start_matches("Bearer ")
                    .to_string();
                if token == "inst-22-token" {
                    (
                        axum::http::StatusCode::OK,
                        Json(serde_json::json!({"total_count": 0, "repositories": []})),
                    )
                        .into_response()
                } else {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"message": "bad credentials"})),
                    )
                        .into_response()
                }
            }),
        )
        // Actions Service mock for the REAL runner access probes.
        .route(
            "/orgs/Indexyz/actions/runners/registration-token",
            post(|| async {
                (
                    axum::http::StatusCode::CREATED,
                    Json(serde_json::json!({"token": "registration"})),
                )
            }),
        )
        .route(
            "/actions/runner-registration",
            post(move || {
                let service = service.clone();
                let admin_token = admin_token.clone();
                async move {
                    Json(serde_json::json!({"url": service, "token": admin_token}))
                }
            }),
        )
        .route(
            "/actions/_apis/runtime/runnergroups/",
            get(move || async move {
                if deny_runner_groups {
                    (
                        axum::http::StatusCode::FORBIDDEN,
                        Json(serde_json::json!({"message": "denied"})),
                    )
                } else {
                    (
                        axum::http::StatusCode::OK,
                        Json(serde_json::json!({"count": 1, "value": [{"id": 7, "name": "Default"}]})),
                    )
                }
            }),
        );
    let routes = crate::auth_worker_mock_routes::routes();
    let app = app.merge(routes.router);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Mock {
        base,
        user_agents,
        metadata_mints,
        discovery_reads,
        repo_id: routes.repo_id,
        repo_owner_id: routes.repo_owner_id,
        scale_set_creates: routes.scale_set_creates,
        installation_reads: routes.installation_reads,
    }
}

pub(crate) fn endpoints(base: &str) -> crate::auth_worker_probe::WorkerEndpoints {
    crate::auth_worker_probe::WorkerEndpoints {
        api_base: base.to_string(),
        allow_test_endpoints: true,
    }
}

/// Builds a promotion payload with a snapshot consistent with `dependents`
/// (used by the promotion-guard test).
pub(crate) fn promotion_from(
    key: &str,
    revision: i64,
    dependents: &[shaula_core::registry::AuthDependentTarget],
) -> shaula_core::registry::AuthPromotion {
    use shaula_core::registry::{
        auth_dependent_set_fingerprint, AuthCheckedFleet, AuthValidationSnapshot,
    };
    shaula_core::registry::AuthPromotion {
        bindings: Vec::new(),
        snapshot_json: serde_json::to_string(&AuthValidationSnapshot {
            candidate: (key.to_string(), revision),
            dependent_set: auth_dependent_set_fingerprint(dependents),
            checked_fleets: dependents
                .iter()
                .map(|d| AuthCheckedFleet {
                    key: d.fleet_key.clone(),
                    incarnation: d.incarnation.clone(),
                    revision: d.revision,
                    fence: d.fence,
                })
                .collect(),
            identities: Vec::new(),
        })
        .unwrap(),
    }
}
