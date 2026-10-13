//! Pool Fleet driver rules shared by every Pool backend (Forgejo, GitLab):
//! capacity arithmetic, the idle deadline, Retirement and expiry selection,
//! and the status phase. Backends own only demand proof, inventory identity,
//! registration and readiness (ARD-0043, amended).

use std::time::Duration;

use shaula_core::capacity::CapacityPolicy;
use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::AccessFailure;
use shaula_core::registry::{
    ControlPlaneStore, FleetObservationPhase, FleetRuntimeGuard, GenerationRecord, LifecycleStore,
};

use crate::runner_operation::{DestroyOutcome, DestroyTally, RunnerOperation, RunnerRegistrations};

/// A Pool Fleet's waiting runner holds infrastructure; the idle deadline is
/// part of its safety boundary (CONTEXT.md: Pool Fleet).
pub(crate) const IDLE_DEADLINE_MS: i64 = 600_000;

pub(crate) fn target(capacity: CapacityPolicy, demand: u64) -> u64 {
    u64::try_from(capacity.min_runners)
        .unwrap_or(0)
        .saturating_add(demand)
        .min(u64::try_from(capacity.max_runners).unwrap_or(0))
}

pub(crate) fn create_count(
    capacity: CapacityPolicy,
    target: u64,
    effective: i64,
    occupancy: i64,
) -> u64 {
    target
        .saturating_sub(u64::try_from(effective).unwrap_or(u64::MAX))
        .min(u64::try_from(capacity.max_runners.saturating_sub(occupancy)).unwrap_or(0))
}

pub(crate) fn access_reason(failure: &AccessFailure) -> ReasonCode {
    match failure {
        AccessFailure::Unauthenticated => ReasonCode::Unauthenticated,
        AccessFailure::PermissionDenied => ReasonCode::PermissionDenied,
        AccessFailure::TargetHiddenOrNotFound => ReasonCode::TargetHiddenOrNotFound,
        AccessFailure::RateLimited { .. } => ReasonCode::RateLimited,
        _ => ReasonCode::AccessVerificationFailed,
    }
}

/// One pass's facts that decide the replacement status phase.
pub(crate) struct PassFacts {
    pub stale: bool,
    pub reason: Option<ReasonCode>,
    pub target: u64,
    /// Owned registrations that are idle-and-serving or running a job.
    pub serving: u64,
}

/// Ready only when every occupying Generation is current, Idle or Busy, and
/// both they and the serving registrations equal the target.
pub(crate) fn phase(
    generations: &[GenerationRecord],
    desired_revision: i64,
    facts: &PassFacts,
) -> (FleetObservationPhase, Option<ReasonCode>) {
    let quarantined = generations.iter().any(|g| g.state.is_quarantined());
    let occupying = generations.iter().filter(|g| g.state.counts_occupancy());
    let converging = occupying.clone().any(|g| {
        g.fleet_revision != desired_revision
            || !matches!(g.state, GenerationState::Idle | GenerationState::Busy)
    });
    let reason = facts
        .reason
        .or(quarantined.then_some(ReasonCode::OwnershipProofFailed));
    let phase = if facts.stale || reason.is_some() {
        FleetObservationPhase::Degraded
    } else if !converging
        && occupying.count() as u64 == facts.target
        && facts.serving == facts.target
    {
        FleetObservationPhase::Ready
    } else {
        FleetObservationPhase::Reconciling
    };
    (phase, reason)
}

/// Retirement and Runner Maximum Lifetime selection for one Pool Fleet. The
/// Destroy rules (state walk, proofs, Quarantine) live in the operation.
pub(crate) struct Retirement<'a, R> {
    pub operation: RunnerOperation<'a, R>,
    pub fleet_key: &'a str,
    pub guard: &'a FleetRuntimeGuard,
    pub lifecycle: &'a dyn LifecycleStore,
    pub store: &'a dyn ControlPlaneStore,
    pub destroy_limit: &'a tokio::sync::Semaphore,
}

impl<R: RunnerRegistrations> Retirement<'_, R> {
    pub(crate) async fn destroy_excess(&self, target: u64, now: i64) -> CoreResult<DestroyTally> {
        let generations = self.lifecycle.generations_for_fleet(self.fleet_key).await?;
        let effective = generations
            .iter()
            .filter(|g| g.state.counts_effective())
            .count() as u64;
        let mut idle_removals = effective.saturating_sub(target);
        let mut tally = DestroyTally::default();
        for generation in generations {
            let resumable = matches!(
                generation.state,
                GenerationState::CleanupRequired
                    | GenerationState::Retiring
                    | GenerationState::DestroyPending
                    | GenerationState::Destroying
            );
            let idle = generation.state == GenerationState::Idle
                && (idle_removals > 0
                    || generation.fleet_revision != self.guard.desired_revision
                    || now.saturating_sub(generation.updated_at) >= IDLE_DEADLINE_MS);
            let retiring_revision = generation.state == GenerationState::WaitingOnline
                && generation.fleet_revision != self.guard.desired_revision;
            if !resumable && !idle && !retiring_revision {
                continue;
            }
            let _permit = self.slot(&generation, now).await?;
            let outcome = self.operation.retire(&generation, now).await;
            if matches!(outcome, Ok(DestroyOutcome::Destroyed)) {
                idle_removals = idle_removals.saturating_sub(u64::from(idle));
            }
            tally.record(&generation, &outcome);
        }
        Ok(tally)
    }

    /// Hard lifetime is not proactive idle drain: terminating active jobs is
    /// intentional.
    pub(crate) async fn expire(&self, now: i64, limit: Duration) -> CoreResult<DestroyTally> {
        let mut tally = DestroyTally::default();
        for generation in self.operation.expired(self.fleet_key, now, limit).await? {
            let _permit = self.slot(&generation, now).await?;
            let outcome = self.operation.expire(&generation, now).await;
            tally.record(&generation, &outcome);
        }
        Ok(tally)
    }

    async fn slot(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<tokio::sync::SemaphorePermit<'_>> {
        crate::diagnostic_capture::acquire_destroy_slot(
            self.destroy_limit,
            self.store,
            Some(self.guard),
            generation,
            now,
        )
        .await
    }
}
