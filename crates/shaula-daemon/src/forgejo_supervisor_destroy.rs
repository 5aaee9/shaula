//! Forgejo Pool Retirement and Runner Maximum Lifetime selection. The Destroy
//! rules (state walk, proofs, Quarantine) live in [`crate::runner_operation`].

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState;

use super::ForgejoPoolSupervisor;
use crate::runner_operation::forgejo::ForgejoRegistrations;
use crate::runner_operation::{DestroyOutcome, DestroyTally, RunnerOperation};

/// A Pool Fleet's waiting runner holds infrastructure; the idle deadline is
/// part of its safety boundary (CONTEXT.md: Pool Fleet).
const IDLE_DEADLINE_MS: i64 = 600_000;

impl ForgejoPoolSupervisor {
    fn runner_operation(&self) -> RunnerOperation<'_, ForgejoRegistrations<'_>> {
        RunnerOperation {
            diagnostic_guard: Some(&self.guard),
            lifecycle: self.lifecycle.as_ref(),
            store: self.store.as_ref(),
            runtime: self.runtime.as_ref(),
            clock: Some(self.clock.as_ref()),
            artifact_root: &self.artifact_root,
            operation_timeout: self.operation_timeout,
            apply_intent: &self.apply_intent_sink,
            provider: shaula_core::fleet::FleetProviderKind::Forgejo,
            registrations: ForgejoRegistrations {
                fleet_key: &self.fleet_key,
                guard: &self.guard,
                lifecycle: self.lifecycle.as_ref(),
                store: self.store.as_ref(),
                gates: &self.gates,
                forgejo: self.forgejo.as_ref(),
                runtime: self.runtime.as_ref(),
                labels: &self.labels,
            },
        }
    }

    pub(super) async fn destroy_excess(&self, target: u64, now: i64) -> CoreResult<DestroyTally> {
        let operation = self.runner_operation();
        let generations = self
            .lifecycle
            .generations_for_fleet(&self.fleet_key)
            .await?;
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
            let _permit = crate::diagnostic_capture::acquire(
                &self.destroy_limit,
                crate::diagnostic_capture::generation_observer(
                    self.store.as_ref(),
                    Some(&self.guard),
                    &generation,
                    shaula_core::diagnostics::Lane::Operation,
                    shaula_core::diagnostics::QuestionId::Cleanup,
                ),
                shaula_core::diagnostics::Code::ExecutionDestroySlotWait,
                now,
            )
            .await
            .map_err(|_| CoreError::new(ReasonCode::Internal, "destroy scheduler stopped"))?;
            let outcome = operation.retire(&generation, now).await;
            if matches!(outcome, Ok(DestroyOutcome::Destroyed)) {
                idle_removals = idle_removals.saturating_sub(u64::from(idle));
            }
            tally.record(&generation, &outcome);
        }
        Ok(tally)
    }

    /// Hard lifetime is not proactive idle drain: terminating active jobs is
    /// intentional.
    pub(super) async fn expire_runners(&self, now: i64) -> CoreResult<DestroyTally> {
        let operation = self.runner_operation();
        let mut tally = DestroyTally::default();
        for generation in operation
            .expired(&self.fleet_key, now, self.runner_max_lifetime)
            .await?
        {
            let _permit = crate::diagnostic_capture::acquire(
                &self.destroy_limit,
                crate::diagnostic_capture::generation_observer(
                    self.store.as_ref(),
                    Some(&self.guard),
                    &generation,
                    shaula_core::diagnostics::Lane::Operation,
                    shaula_core::diagnostics::QuestionId::Cleanup,
                ),
                shaula_core::diagnostics::Code::ExecutionDestroySlotWait,
                now,
            )
            .await
            .map_err(|_| CoreError::new(ReasonCode::Internal, "destroy scheduler stopped"))?;
            let outcome = operation.expire(&generation, now).await;
            tally.record(&generation, &outcome);
        }
        Ok(tally)
    }
}
