//! GitHub Retirement and Runner Maximum Lifetime selection. The Destroy rules
//! (state walk, proofs, Quarantine) live in [`crate::runner_operation`].

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState as G;

use super::FleetSupervisor;
use crate::runner_operation::github::GithubRegistrations;
use crate::runner_operation::{DestroyTally, RunnerOperation};

/// Spec 0024 §2.1: a CleanupRequired Generation joins the Destroy chain after
/// a one-tick grace.
const CLEANUP_GRACE_MS: i64 = 60_000;

impl FleetSupervisor {
    fn runner_operation(&self) -> RunnerOperation<'_, GithubRegistrations<'_>> {
        RunnerOperation {
            lifecycle: self.store.as_ref(),
            store: self.handoff.as_ref(),
            runtime: self.runtime.as_ref(),
            clock: self.clock.as_deref(),
            artifact_root: &self.config.artifact_root,
            operation_timeout: self.config.operation_timeout,
            apply_intent: &self.config.apply_intent_sink,
            provider: shaula_core::fleet::FleetProviderKind::Github,
            registrations: GithubRegistrations {
                fleet_key: &self.config.fleet_key,
                lifecycle: self.store.as_ref(),
                store: self.handoff.as_ref(),
                current: (&self.config.auth_profile_key, self.config.auth_revision),
                github: &self.github,
                revision_clients: &self.revision_clients,
            },
        }
    }

    /// Retires up to `excess` Idle Generations and resumes every in-flight
    /// Retirement. Resumable Generations are re-selected on every tick, so a
    /// failed or crashed Destroy continues from its durable state.
    pub(crate) async fn retire_excess(
        &self,
        mut excess: i64,
        now: i64,
    ) -> CoreResult<DestroyTally> {
        let operation = self.runner_operation();
        let mut tally = DestroyTally::default();
        for generation in self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?
        {
            let selected = match generation.state {
                G::Retiring | G::DestroyPending | G::Destroying => true,
                G::CleanupRequired => now.saturating_sub(generation.updated_at) >= CLEANUP_GRACE_MS,
                G::Idle => excess > 0,
                _ => false,
            };
            if !selected {
                continue;
            }
            if generation.state == G::Idle {
                excess -= 1;
            }
            let _permit =
                self.limits.destroy.acquire().await.map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "destroy scheduler stopped")
                })?;
            let outcome = operation.retire(&generation, now).await;
            tally.record(&generation, &outcome);
        }
        Ok(tally)
    }

    /// Hard lifetime is independent of demand, listener health and handoff.
    pub(super) async fn expire_runners(&self, now: i64) -> CoreResult<DestroyTally> {
        let operation = self.runner_operation();
        let mut tally = DestroyTally::default();
        for generation in operation
            .expired(&self.config.fleet_key, now, self.runner_max_lifetime)
            .await?
        {
            let _permit =
                self.limits.destroy.acquire().await.map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "destroy scheduler stopped")
                })?;
            let outcome = operation.expire(&generation, now).await;
            tally.record(&generation, &outcome);
        }
        Ok(tally)
    }
}
