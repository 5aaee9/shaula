//! Composition fixture entrypoint; production policy lives in shaula-daemon.
use shaula_core::{
    error::CoreResult,
    ports::Clock,
    registry::{AuthRevisionRow, ControlPlaneStore},
};
pub(crate) use shaula_daemon::auth_validation::github::Verdict;
use std::sync::Arc;
pub(crate) async fn validate_v2(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    row: &AuthRevisionRow,
    endpoints: &super::auth_worker_probe::WorkerEndpoints,
) -> CoreResult<Verdict> {
    let adapter =
        shaula_scaleset::validation::GitHubValidation::new(endpoints.clone(), clock.clone())?;
    shaula_daemon::auth_validation::github::validate_v2(store, clock, key, row, &adapter).await
}
#[path = "auth_worker_v2_tests.rs"]
pub(crate) mod tests;
