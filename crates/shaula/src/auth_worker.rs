//! Asynchronous Auth Candidate validation (spec 0005 §6, spec 0011 §4.1).
//! The dispatch splits by stored schema version: legacy single-
//! installation revisions keep the exact prior flow; v2 policy revisions
//! run the multi-account flow in `auth_worker_v2`.

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
    if row.schema_version >= 2 {
        return match crate::auth_worker_v2::validate_v2(&store, &clock, &key, &row, endpoints)
            .await?
        {
            crate::auth_worker_v2::Verdict::Accepted | crate::auth_worker_v2::Verdict::Rejected => {
                Ok(WorkerFlow::Done)
            }
            crate::auth_worker_v2::Verdict::RetryNeeded { retry_after_ms } => {
                // Honor GitHub's Retry-After deadline in scheduling (F8):
                // the wiring defers the next worker attempt until then.
                Ok(WorkerFlow::Deferred {
                    retry_at_unix_ms: retry_after_ms
                        .map(|ms| clock.now_unix_ms().saturating_add(ms)),
                })
            }
        };
    }
    let allowlist: shaula_core::auth::TargetAllowlist =
        serde_json::from_str(&row.allowlist_json)
            .map_err(|_| CoreError::new(ReasonCode::Internal, "stored auth allowlist invalid"))?;
    let Some(credential) =
        crate::wiring_credential::build_credential(&store, &key, row.revision, None).await?
    else {
        return Ok(WorkerFlow::Done);
    };
    let mut accepted = !allowlist.targets.is_empty();
    for target in allowlist.targets {
        let client = endpoints.probe_client(target, credential.clone(), clock.clone())?;
        if let Err(error) = client.validate_auth(&row).await {
            match error {
                shaula_scaleset::ScalesetError::RateLimited {
                    retry_after_secs, ..
                } => {
                    return Ok(WorkerFlow::Deferred {
                        retry_at_unix_ms: retry_after_secs.map(|s| {
                            clock
                                .now_unix_ms()
                                .saturating_add(s.max(0).saturating_mul(1000))
                        }),
                    });
                }
                shaula_scaleset::ScalesetError::Configuration { .. }
                | shaula_scaleset::ScalesetError::Status {
                    status: 401 | 403 | 404,
                    ..
                } => {
                    accepted = false;
                    break;
                }
                _ => {
                    return Err(CoreError::new(
                        ReasonCode::AccessVerificationFailed,
                        "auth validation temporarily unavailable",
                    ))
                }
            }
        }
    }
    store
        .auth_apply_validation_v2(
            &key,
            row.revision,
            accepted,
            if accepted {
                None
            } else {
                Some("IdentityOrAccessVerificationFailed")
            },
            clock.now_unix_ms(),
            None,
        )
        .await
        .map(|_| WorkerFlow::Done)
}
