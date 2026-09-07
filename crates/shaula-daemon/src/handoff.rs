//! Auth handoff state machine: quiesce, read-only ownership/absence
//! classification, then acknowledge the full desired tuple. The handoff
//! never creates/adopts a Scale Set, binds an ID, establishes a session or
//! mints JIT — ordinary reconciliation owns those effects exclusively.

use std::sync::Arc;

use shaula_core::error::CoreResult;
use shaula_core::ports::AccessFailure;
use shaula_core::ports::GitHubAccessPort;
use shaula_core::registry::ControlPlaneStore;

/// Outcome of one handoff attempt for one fleet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffProgress {
    /// desired == observed already; nothing to do.
    UpToDate,
    /// Observed tuple advanced to the full desired tuple.
    Acknowledged,
    /// Blocked with a durable retry deadline; never falls back.
    Blocked,
}

/// Runs one durable handoff attempt against the exact desired tuple.
pub async fn run_handoff(
    store: &Arc<dyn ControlPlaneStore>,
    fleet_key: &str,
    github: &Arc<dyn GitHubAccessPort>,
    identity: &shaula_core::github::ScaleSetIdentity,
    bound_scale_set_id: Option<i64>,
    now: i64,
    backoff_secs: i64,
) -> CoreResult<HandoffProgress> {
    let Some(handoff) = store.handoff_get(fleet_key).await? else {
        return Ok(HandoffProgress::UpToDate);
    };
    if handoff.observed == Some(handoff.desired.clone()) {
        return Ok(HandoffProgress::UpToDate);
    }
    // Persisted backoff: a Blocked handoff must not hammer GitHub every
    // tick.
    if let Some(retry_at) = handoff.retry_at {
        if now < retry_at {
            return Ok(HandoffProgress::Blocked);
        }
    }

    let (desired_key, desired_revision) = handoff.desired.clone();
    // The GitHub port already carries the credential matching the desired
    // tuple; a kind or revision mismatch is an access failure, never a
    // fallback trigger. R10-03: the ownership proof also checks the
    // BOUND durable scale set id and runner group — an exactly-one result
    // alone proves nothing about WHICH set answered.
    let classification = classify_ownership(github, identity, bound_scale_set_id).await;
    match classification {
        OwnershipClassification::AccessVerified | OwnershipClassification::ScaleSetAbsent => {
            store
                .handoff_acknowledge(fleet_key, &desired_key, desired_revision)
                .await?;
            Ok(HandoffProgress::Acknowledged)
        }
        OwnershipClassification::Blocked(reason) => {
            store
                .handoff_mark_blocked(fleet_key, &reason, now + backoff_secs * 1000)
                .await?;
            Ok(HandoffProgress::Blocked)
        }
    }
}

/// Read-only ownership/absence classification for the persisted Scale Set
/// identity. Access failures (`401`/`403`/filtered `404`) block; only a
/// successful authenticated read proving absence yields `ScaleSetAbsent`.
/// When a scale set IS durably bound (`bound_id`), an exactly-one result
/// must carry THAT id and the resolved runner group — any drift blocks
/// instead of acknowledging (R10-03).
enum OwnershipClassification {
    AccessVerified,
    ScaleSetAbsent,
    Blocked(String),
}

async fn classify_ownership(
    github: &Arc<dyn GitHubAccessPort>,
    identity: &shaula_core::github::ScaleSetIdentity,
    bound_id: Option<i64>,
) -> OwnershipClassification {
    match github.resolve_runner_group(identity).await {
        Ok(group_id) => match github.lookup_scale_set(identity, group_id).await {
            Ok(shaula_core::ports::LookupOutcome::ExactlyOne(view)) => {
                if bound_id.is_some_and(|id| id != view.id) {
                    OwnershipClassification::Blocked(
                        "bound scale set id drifted from the authoritative lookup".into(),
                    )
                } else if view.runner_group_id != group_id {
                    OwnershipClassification::Blocked(
                        "scale set runner group drifted from the identity".into(),
                    )
                } else {
                    OwnershipClassification::AccessVerified
                }
            }
            Ok(shaula_core::ports::LookupOutcome::None) => OwnershipClassification::ScaleSetAbsent,
            Ok(shaula_core::ports::LookupOutcome::Multiple) => {
                OwnershipClassification::Blocked("multiple scale sets match the identity".into())
            }
            Err(failure) => OwnershipClassification::Blocked(failure.summary()),
        },
        Err(AccessFailure::TargetHiddenOrNotFound) => {
            OwnershipClassification::Blocked("runner group hidden or not found".into())
        }
        Err(failure) => OwnershipClassification::Blocked(failure.summary()),
    }
}

#[cfg(test)]
#[path = "handoff_tests.rs"]
mod handoff_tests;
