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
    /// The in-flight completion lost a CAS race: abort this tick and
    /// retry from the CURRENT authority — never treated as settlement
    /// (G5).
    Retry,
}

/// Runs one durable handoff attempt against the exact desired tuple.
pub async fn run_handoff(
    store: &Arc<dyn ControlPlaneStore>,
    fleet_key: &str,
    github: &Arc<dyn GitHubAccessPort>,
    authority: &(String, i64),
    identity: &shaula_core::github::ScaleSetIdentity,
    bound_scale_set_id: Option<i64>,
    now: i64,
) -> CoreResult<HandoffProgress> {
    let backoff_secs = 30;
    let Some(handoff) = store.handoff_get(fleet_key).await? else {
        return Ok(HandoffProgress::UpToDate);
    };
    if &handoff.desired != authority {
        return Ok(HandoffProgress::Retry);
    }
    if handoff.observed == Some(handoff.desired.clone()) {
        // Ref equality alone is not authority (F7): a v2 desired revision
        // also requires its observed context before the handoff settles.
        let context_settled = observed_context_settled(store, fleet_key, &handoff.desired).await?;
        if context_settled {
            return Ok(HandoffProgress::UpToDate);
        }
    }
    // Persisted backoff: a Blocked handoff must not hammer GitHub every
    // tick.
    if let Some(retry_at) = handoff.retry_at {
        if now < retry_at {
            return Ok(HandoffProgress::Blocked);
        }
    }

    let (desired_key, desired_revision) = handoff.desired.clone();
    // G5: capture the fence BEFORE any network validation — the ack must
    // still find this fence after the remote round trip.
    let Some(fleet) = store.fleet_get(fleet_key).await? else {
        return Ok(HandoffProgress::Retry);
    };
    let desired_context = store.fleet_auth_context_get(fleet_key).await?;
    let expectation = shaula_core::registry::AuthHandoffExpectation {
        mutation_fence: fleet.mutation_fence,
        desired_context_json: desired_context
            .as_ref()
            .filter(|row| row.desired.as_ref() == Some(&handoff.desired))
            .and_then(|row| row.desired_context_json.clone()),
    };
    if let Err(failure) = github.ensure_route_proof().await {
        return record_failure(
            store,
            fleet_key,
            authority,
            &expectation,
            &failure.summary(),
            now + backoff_secs * 1000,
        )
        .await;
    }
    // The GitHub port already carries the credential matching the desired
    // tuple; a kind or revision mismatch is an access failure, never a
    // fallback trigger. R10-03: the ownership proof also checks the
    // BOUND durable scale set id and runner group — an exactly-one result
    // alone proves nothing about WHICH set answered.
    let classification = classify_ownership(github, identity, bound_scale_set_id).await;
    match classification {
        OwnershipClassification::AccessVerified | OwnershipClassification::ScaleSetAbsent => {
            // v2 (spec 0011 §5.2): verify the exact Resolved Auth Context
            // FIRST, then advance observed ref + context ATOMICALLY. Only
            // an equal ref tuple is not enough to prove the context
            // switched.
            match resolve_exact_context(
                store,
                github,
                fleet_key,
                &desired_key,
                desired_revision,
                expectation.desired_context_json.as_deref(),
                &identity.target,
            )
            .await?
            {
                ExactContext::None => {
                    match store
                        .handoff_acknowledge(
                            fleet_key,
                            &desired_key,
                            desired_revision,
                            None,
                            &expectation,
                        )
                        .await?
                    {
                        shaula_core::registry::FleetContextAck::Stale => {
                            // A genuinely stale CAS aborts this tick and
                            // retries from CURRENT authority (G5) — it is
                            // never successful settlement.
                            Ok(HandoffProgress::Retry)
                        }
                        _ => Ok(HandoffProgress::Acknowledged),
                    }
                }
                ExactContext::Verified(context_json) => {
                    match store
                        .handoff_acknowledge(
                            fleet_key,
                            &desired_key,
                            desired_revision,
                            Some(&context_json),
                            &expectation,
                        )
                        .await?
                    {
                        shaula_core::registry::FleetContextAck::Acknowledged
                        | shaula_core::registry::FleetContextAck::NotApplicable => {
                            Ok(HandoffProgress::Acknowledged)
                        }
                        shaula_core::registry::FleetContextAck::Stale => Ok(HandoffProgress::Retry),
                        shaula_core::registry::FleetContextAck::IdentityDrift => {
                            // A same-name rebuild or ownership transfer is
                            // never silently accepted; evidence is kept.
                            record_failure(
                                store,
                                fleet_key,
                                authority,
                                &expectation,
                                "TargetIdentityChanged",
                                now + backoff_secs * 1000,
                            )
                            .await
                        }
                    }
                }
                ExactContext::Blocked(reason) => {
                    record_failure(
                        store,
                        fleet_key,
                        authority,
                        &expectation,
                        &reason,
                        now + backoff_secs * 1000,
                    )
                    .await
                }
            }
        }
        OwnershipClassification::Blocked(reason) => {
            record_failure(
                store,
                fleet_key,
                authority,
                &expectation,
                &reason,
                now + backoff_secs * 1000,
            )
            .await
        }
    }
}

async fn record_failure(
    store: &Arc<dyn ControlPlaneStore>,
    fleet_key: &str,
    authority: &(String, i64),
    expectation: &shaula_core::registry::AuthHandoffExpectation,
    reason: &str,
    retry_at: i64,
) -> CoreResult<HandoffProgress> {
    if store
        .handoff_mark_blocked(fleet_key, authority, expectation, reason, retry_at)
        .await?
    {
        Ok(HandoffProgress::Blocked)
    } else {
        Ok(HandoffProgress::Retry)
    }
}

/// The exact-context resolution outcome for one handoff attempt.
enum ExactContext {
    /// Legacy revision (or no context intent): ref-only ack.
    None,
    /// The verified context JSON to persist atomically with the ref.
    Verified(String),
    /// A durable block reason; nothing is written.
    Blocked(String),
}

/// Whether the observed side of a fleet's context rollout already
/// matches the desired ref (F7). Legacy profiles (no v2 context intent)
/// settle on ref equality alone.
async fn observed_context_settled(
    store: &Arc<dyn ControlPlaneStore>,
    fleet_key: &str,
    desired: &(String, i64),
) -> CoreResult<bool> {
    let Some(revision_row) = store.auth_revision_get(&desired.0, desired.1).await? else {
        return Ok(false);
    };
    if revision_row.schema_version < 2 {
        return Ok(true);
    }
    let Some(context_row) = store.fleet_auth_context_get(fleet_key).await? else {
        return Ok(false);
    };
    let Some(json) = context_row.observed_context_json else {
        return Ok(false);
    };
    let context: shaula_core::auth_context::ResolvedAuthContext = serde_json::from_str(&json)
        .map_err(|e| {
            shaula_core::error::CoreError::new(
                shaula_core::error::ReasonCode::Internal,
                format!("observed auth context corrupt: {e}"),
            )
        })?;
    Ok(context_row.observed.as_ref() == Some(desired)
        && context.profile_key == desired.0
        && context.revision == desired.1
        && context.has_complete_identity())
}

/// Builds the verified exact Resolved Auth Context for a v2 desired
/// revision (spec 0011 §5.2 step 2/4): the desired intent from admission
/// is completed with the REMOTE numeric identity of the Target. A target
/// that no longer matches the fleet's saved identity — or a GitHub that
/// cannot prove it — blocks; nothing is written unverified.
async fn resolve_exact_context(
    store: &Arc<dyn ControlPlaneStore>,
    github: &Arc<dyn GitHubAccessPort>,
    _fleet_key: &str,
    profile_key: &str,
    revision: i64,
    desired_json: Option<&str>,
    fleet_target: &shaula_core::github::GitHubTarget,
) -> CoreResult<ExactContext> {
    let Some(revision_row) = store.auth_revision_get(profile_key, revision).await? else {
        return Ok(ExactContext::Blocked("AuthRevisionMissing".into()));
    };
    if revision_row.schema_version < 2 {
        return Ok(ExactContext::None);
    }
    let Some(json) = desired_json else {
        return Ok(ExactContext::Blocked("AuthContextMissing".into()));
    };
    let Ok(mut context) =
        serde_json::from_str::<shaula_core::auth_context::ResolvedAuthContext>(json)
    else {
        return Ok(ExactContext::Blocked("AuthContextCorrupt".into()));
    };
    if context.target != *fleet_target
        || context.profile_key != profile_key
        || context.revision != revision
    {
        return Ok(ExactContext::Blocked("TargetIdentityChanged".into()));
    }
    // The remote numeric identity is the verification: organization id,
    // or repository id + owner id for repository Targets.
    match github.resolve_target_identity(&context.target).await {
        Ok(ids) => {
            let desired = context.clone();
            context.organization_id = ids.organization_id;
            context.repository_id = ids.repository_id;
            context.repository_owner_id = ids.repository_owner_id;
            if !context.completes(&desired) {
                return Ok(ExactContext::Blocked("TargetIdentityChanged".into()));
            }
        }
        Err(failure) => return Ok(ExactContext::Blocked(failure.summary())),
    }
    match serde_json::to_string(&context) {
        Ok(verified) => Ok(ExactContext::Verified(verified)),
        Err(e) => Err(shaula_core::error::CoreError::new(
            shaula_core::error::ReasonCode::Internal,
            e.to_string(),
        )),
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
#[cfg(test)]
#[path = "handoff_v2_tests.rs"]
mod handoff_v2_tests;
