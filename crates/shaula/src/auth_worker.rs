//! Asynchronous Auth Candidate validation (spec 0005 §6, spec 0011 §4.1).
//! Only schema-v2 GitHub App revisions may enter validation.

use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    ports::Clock,
    registry::ControlPlaneStore,
};
use std::sync::Arc;

/// Bounded default backoff (G2) when a transient failure supplies no
/// Retry-After deadline: the worker re-attempts after this interval —
/// never deferred for the rest of the process.
pub(crate) const DEFAULT_RETRY_BACKOFF_MS: i64 = 30_000;
/// The scheduling outcome of one worker pass (F8): a rate-limited flow
/// carries the ABSOLUTE retry deadline GitHub supplied so the wiring can
/// defer the next attempt instead of polling every scan interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkerFlow {
    /// Terminal for now: promoted, rejected, or nothing to validate.
    Done,
    /// Stay Pending; do not re-attempt before this deadline (unix ms).
    Deferred { retry_at_unix_ms: Option<i64> },
}

pub(super) async fn validate(
    store: Arc<dyn ControlPlaneStore>,
    clock: Arc<dyn Clock>,
    key: String,
    revision: i64,
    endpoints: &crate::auth_worker_probe::WorkerEndpoints,
) -> CoreResult<WorkerFlow> {
    let Some(head) = store.auth_profile_get(&key).await? else {
        return Ok(WorkerFlow::Done);
    };
    if head.status != "Validating" || head.desired_revision != revision {
        return Ok(WorkerFlow::Done);
    }
    let Some(row) = store.auth_revision_get(&key, head.desired_revision).await? else {
        return Ok(WorkerFlow::Done);
    };
    if row.schema_version != 2 || row.kind != "github_app" {
        return Err(CoreError::new(
            ReasonCode::CredentialMalformed,
            "unsupported authentication revision",
        ));
    }
    match crate::auth_worker_v2::validate_v2(&store, &clock, &key, &row, endpoints).await? {
        crate::auth_worker_v2::Verdict::Accepted | crate::auth_worker_v2::Verdict::Rejected => {
            Ok(WorkerFlow::Done)
        }
        crate::auth_worker_v2::Verdict::RetryNeeded { retry_after_ms } => {
            // Honor GitHub's Retry-After deadline in scheduling (F8):
            // the wiring defers the next worker attempt until then.
            Ok(WorkerFlow::Deferred {
                retry_at_unix_ms: retry_after_ms.map(|ms| clock.now_unix_ms().saturating_add(ms)),
            })
        }
    }
}
