//! Runner Operation: the Runner Backend-neutral Create/Destroy rules for one
//! Runner Generation (CONTEXT.md). Drivers select candidates; this Module owns
//! the Generation state walk, the destroy proof requirements and the Quarantine
//! rules. Backend differences sit behind [`RunnerRegistrations`].
//!
//! Failure classes (ARD-0040, spec 0024 §2.1): a temporarily unobservable fact
//! returns `Err`/[`DestroyOutcome::Pending`] and retries on a later tick; missing
//! proof quarantines; one Generation's failure never aborts a whole pass.

mod destroy;
pub(crate) mod forgejo;
pub(crate) mod github;
mod lifetime;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use shaula_core::error::CoreResult;
use shaula_core::fleet::FleetProviderKind;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::{ApplyIntentSink, Clock, TemplateRuntimePort};
use shaula_core::registry::{ControlPlaneStore, GenerationRecord, LifecycleStore};

/// Verdict of one Runner Registration question asked across the Runner
/// Backend seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Registration {
    /// Removal may start now: record the Retirement intent, then remove.
    Removable,
    /// Proven removed, or proven never present.
    Gone,
    /// The backend safety gate refuses removal now; retry on a later tick.
    Busy,
    /// Remote observation or authority is temporarily unavailable; retry.
    Unavailable,
    /// The identity is contradicted or the removal authority is missing.
    Unprovable,
    /// No Runner Registration identity is recorded for the Generation.
    Unrecorded,
}

/// The removal guarantee a Destroy reason requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemovalGate {
    /// Retirement: never remove a registration that may still hold a job.
    Retirement { create_started: bool },
    /// Runner Maximum Lifetime: the Runner Resource is already destroyed and
    /// terminating a running job is intentional.
    AfterResourcesDestroyed,
}

/// The Runner Backend seam: the registration questions a Runner Operation
/// asks. GitHub and Forgejo are the two adapters.
///
/// Retirement is recorded only once removal is admitted. What "admitted"
/// means is backend-specific: GitHub's busy-safe gate is the removal call
/// itself (a refused one-job runner is already fenced by its job), while
/// Forgejo must prove idleness read-only first because an unproven runner
/// can still acquire a job.
pub(crate) trait RunnerRegistrations: Sync {
    /// Read-only Retirement admission: [`Registration::Removable`], `Gone`,
    /// `Busy`, `Unavailable`, `Unprovable` or `Unrecorded`.
    async fn admit_removal(
        &self,
        generation: &GenerationRecord,
        create_started: bool,
    ) -> CoreResult<Registration>;

    /// Removes the recorded registration: `Gone`, `Busy`, `Unavailable`,
    /// `Unprovable`, or `Unrecorded` when no identity is recorded.
    async fn remove(
        &self,
        generation: &GenerationRecord,
        gate: RemovalGate,
    ) -> CoreResult<Registration>;

    /// Proves that a Generation without a recorded identity has no
    /// registration: [`Registration::Gone`], `Unavailable` or `Unprovable`.
    async fn prove_unrecorded_absent(
        &self,
        generation: &GenerationRecord,
    ) -> CoreResult<Registration>;
}

/// Result of one Runner Operation step for a Generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DestroyOutcome {
    Destroyed,
    Pending,
    Quarantined,
}

/// Aggregated outcomes of one driver pass; errors are logged and isolated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DestroyTally {
    pub destroyed: u32,
    pub quarantined: u32,
}

impl DestroyTally {
    pub(crate) fn record(
        &mut self,
        generation: &GenerationRecord,
        outcome: &CoreResult<DestroyOutcome>,
    ) {
        match outcome {
            Ok(DestroyOutcome::Destroyed) => self.destroyed = self.destroyed.saturating_add(1),
            Ok(DestroyOutcome::Quarantined) => {
                self.quarantined = self.quarantined.saturating_add(1);
            }
            Ok(DestroyOutcome::Pending) => {}
            Err(error) => tracing::warn!(
                fleet = %generation.fleet_key,
                generation = %generation.id,
                summary = %error.summary,
                "runner operation step failed; retrying on a later tick"
            ),
        }
    }
}

/// Dependencies of Runner Operation steps for one Fleet; `registrations` is
/// the Runner Backend adapter.
pub(crate) struct RunnerOperation<'a, R> {
    pub diagnostic_guard: Option<&'a shaula_core::registry::FleetRuntimeGuard>,
    pub lifecycle: &'a dyn LifecycleStore,
    pub store: &'a dyn ControlPlaneStore,
    pub runtime: &'a dyn TemplateRuntimePort,
    pub clock: Option<&'a dyn Clock>,
    pub artifact_root: &'a Path,
    pub operation_timeout: Duration,
    pub apply_intent: &'a Arc<dyn ApplyIntentSink>,
    pub provider: FleetProviderKind,
    pub registrations: R,
}

impl<R: RunnerRegistrations> RunnerOperation<'_, R> {
    /// Whether a Create apply was ever admitted: the durable `ApplyStarting`
    /// record precedes every mutating apply (spec 0004 §6).
    async fn create_started(&self, id: &str) -> CoreResult<bool> {
        if self
            .lifecycle
            .operation_original_provenance(id)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        Ok(self
            .lifecycle
            .operations_open_for_generation(id)
            .await?
            .iter()
            .any(|operation| operation.kind == "Create"))
    }

    async fn quarantine(&self, id: &str, now: i64, reason: &str) -> CoreResult<DestroyOutcome> {
        tracing::error!(generation = %id, reason, "safe destroy cannot be proven; quarantining");
        self.lifecycle
            .generation_advance(id, GenerationState::Quarantined, now)
            .await?;
        Ok(DestroyOutcome::Quarantined)
    }

    /// Effect completion time: the injected clock when present.
    fn now_after(&self, now: i64) -> i64 {
        self.clock.map_or(now, Clock::now_unix_ms)
    }
}

mod diagnostics;
