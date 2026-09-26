//! Destroy for Runner Maximum Lifetime: the Runner Resource first, even while
//! a job is running, then the Runner Registration. Ordinary Retirement stays
//! busy-safe; only the ordering differs, never the proof requirements.

use std::time::Duration;

use shaula_core::diagnostics::*;
use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::registry::GenerationRecord;

use super::destroy::ResourceDestroy;
use super::{DestroyOutcome, Registration, RemovalGate, RunnerOperation, RunnerRegistrations};

impl<R: RunnerRegistrations> RunnerOperation<'_, R> {
    /// Generations whose lifetime elapsed or whose expiry is already committed.
    pub(crate) async fn expired(
        &self,
        fleet_key: &str,
        now: i64,
        limit: Duration,
    ) -> CoreResult<Vec<GenerationRecord>> {
        let mut due = Vec::new();
        for generation in self.lifecycle.generations_for_fleet(fleet_key).await? {
            if matches!(
                generation.state,
                G::CreatePending | G::Quarantined | G::Destroyed
            ) {
                continue;
            }
            if self
                .lifecycle
                .generation_lifetime(&generation.id)
                .await?
                .is_due(now, limit)
            {
                due.push(generation);
            }
        }
        Ok(due)
    }

    /// One resource-first expiry step. Registration removal follows and never
    /// gates termination of an overlong job.
    pub(crate) async fn expire(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<DestroyOutcome> {
        let mut diagnostic = self.capture(generation, now);
        if let Some(d) = &mut diagnostic.0 {
            d.observation.question.cleanup_mode = Some(CleanupMode::HardLifetime);
        }
        let id = generation.id.as_str();
        if !self.lifecycle.generation_request_expiry(id, now).await? {
            return Ok(DestroyOutcome::Pending);
        }
        diagnostic.reason(Code::CleanupHardLifetime, StageId::Intent, false);
        let lifetime = self.lifecycle.generation_lifetime(id).await?;
        if lifetime.resources_destroyed_at.is_none() {
            tracing::warn!(fleet = %generation.fleet_key, generation = %id,
                "runner maximum lifetime reached; terminating resources even if a job is running");
            let proof = match self.destroy_proof(generation).await? {
                Ok(proof) => proof,
                Err(reason) => return self.quarantine(id, now, reason).await,
            };
            self.walk(id, generation.state, G::Destroying, now).await?;
            match self.destroy_resources(generation, proof, now).await? {
                ResourceDestroy::Destroyed { at } => {
                    self.lifecycle
                        .generation_resources_destroyed(id, at)
                        .await?;
                }
                ResourceDestroy::Failed { at, known_failure } => {
                    diagnostic.reason(
                        if known_failure {
                            Code::LifecycleOperationFailed
                        } else {
                            Code::LifecycleApplyOutcomeUnknown
                        },
                        StageId::ResourceCleanup,
                        known_failure,
                    );
                    self.lifecycle
                        .generation_advance(id, G::DestroyPending, at)
                        .await?;
                    return Ok(DestroyOutcome::Pending);
                }
            }
        }
        diagnostic.pass(StageId::ResourceCleanup);
        match self
            .registrations
            .remove(generation, RemovalGate::AfterResourcesDestroyed)
            .await?
        {
            Registration::Gone => {
                self.lifecycle
                    .generation_advance(id, G::Destroyed, self.now_after(now))
                    .await?;
                tracing::info!(generation = %id,
                    "hard timeout cleanup complete: resources and registration removed");
                diagnostic.reason(Code::CleanupCompleted, StageId::Completion, false);
                diagnostic.outcome(Outcome::Satisfied);
                if let Some(d) = &mut diagnostic.0 {
                    for r in &mut d.observation.question.reasons {
                        if r.code == Code::CleanupCompleted.as_str() {
                            r.parameters.completion_source =
                                Some(CompletionSource::ProviderCleanup);
                        }
                    }
                }
                Ok(DestroyOutcome::Destroyed)
            }
            Registration::Busy | Registration::Unavailable => {
                diagnostic.reason(
                    Code::CleanupRegistrationPending,
                    StageId::RegistrationCleanup,
                    true,
                );
                tracing::warn!(generation = %id, "expired runner deregistration pending; will retry");
                Ok(DestroyOutcome::Pending)
            }
            // A successful Create always retains its registration identity.
            Registration::Removable | Registration::Unprovable | Registration::Unrecorded => {
                self.quarantine(
                    id,
                    now,
                    "expired runner registration cannot be proven removed",
                )
                .await
            }
        }
    }
}
