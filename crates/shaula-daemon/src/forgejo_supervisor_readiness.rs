//! Inventory readiness and restart classification; a lost Create is never replayed.

use shaula_core::diagnostics::*;
use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::forgejo::ForgejoRunnerRef;
use shaula_core::registry::GenerationRecord;

use super::ForgejoPoolSupervisor;
use crate::runner_operation::forgejo::observe_identity;

use crate::supervisor::READINESS_TIMEOUT_MS;

/// The readiness observation recorded alongside a transition decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadinessNote {
    /// Nothing to record.
    Silent,
    /// The runner inventory stage passed; the question is satisfied.
    Satisfied,
    /// Still waiting for the runner to come online.
    AwaitingOnline,
    /// The readiness deadline elapsed without an active runner.
    TimedOut,
}

/// Pure readiness decision for one WaitingOnline/Idle/Busy generation from
/// its observed runner: the next state (if any) and the diagnostic note.
fn next_state(
    state: GenerationState,
    observed: Result<Option<&ForgejoRunnerRef>, ()>,
    labels: &[String],
    created_at: i64,
    now: i64,
) -> (Option<GenerationState>, ReadinessNote) {
    let runner = match observed {
        Err(()) => return (Some(GenerationState::Quarantined), ReadinessNote::Silent),
        Ok(None) => {
            let next = match state {
                GenerationState::WaitingOnline => GenerationState::CleanupRequired,
                _ => GenerationState::Retiring,
            };
            return (Some(next), ReadinessNote::Silent);
        }
        Ok(Some(runner)) => runner,
    };
    if !runner.is_known() || (!runner.labels.is_empty() && !super::is_owned_runner(runner, labels))
    {
        (Some(GenerationState::Quarantined), ReadinessNote::Silent)
    } else if state == GenerationState::WaitingOnline
        && runner.is_idle()
        && super::is_owned_runner(runner, labels)
    {
        (Some(GenerationState::Idle), ReadinessNote::Satisfied)
    } else if state == GenerationState::Idle && runner.is_active() {
        (Some(GenerationState::Busy), ReadinessNote::Silent)
    } else if state == GenerationState::WaitingOnline
        && now.saturating_sub(created_at) >= READINESS_TIMEOUT_MS
        && !runner.is_active()
    {
        (
            Some(GenerationState::CleanupRequired),
            ReadinessNote::TimedOut,
        )
    } else if state == GenerationState::WaitingOnline {
        (None, ReadinessNote::AwaitingOnline)
    } else if runner.is_idle() || runner.is_active() {
        (None, ReadinessNote::Satisfied)
    } else {
        (None, ReadinessNote::Silent)
    }
}

fn record_note(diagnostic: &mut Capture, note: ReadinessNote) {
    match note {
        ReadinessNote::Silent => {}
        ReadinessNote::Satisfied => {
            diagnostic.pass(StageId::RunnerInventory);
            diagnostic.outcome(Outcome::Satisfied);
        }
        ReadinessNote::AwaitingOnline => {
            diagnostic.reason(
                Code::LifecycleAwaitingOnline,
                StageId::RunnerInventory,
                false,
            );
            diagnostic.outcome(Outcome::Progressing);
        }
        ReadinessNote::TimedOut => {
            diagnostic.reason(
                Code::LifecycleReadinessTimeout,
                StageId::RunnerInventory,
                true,
            );
        }
    }
}

impl ForgejoPoolSupervisor {
    pub(super) async fn reconcile_generation_readiness(
        &self,
        runners: &[ForgejoRunnerRef],
        now: i64,
    ) -> CoreResult<()> {
        for generation in self
            .lifecycle
            .generations_for_fleet(&self.fleet_key)
            .await?
        {
            match generation.state {
                GenerationState::CreatePending => {
                    self.lifecycle
                        .generation_advance(&generation.id, GenerationState::Creating, now)
                        .await?;
                    self.lifecycle
                        .generation_advance(&generation.id, GenerationState::CleanupRequired, now)
                        .await?;
                }
                GenerationState::Creating => self.recover_create(&generation, now).await?,
                GenerationState::WaitingOnline | GenerationState::Idle | GenerationState::Busy => {
                    self.observe_readiness(&generation, runners, now).await?
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Observes one online-phase generation's runner (inventory first,
    /// then a direct lookup) and applies the readiness decision.
    async fn observe_readiness(
        &self,
        generation: &GenerationRecord,
        runners: &[ForgejoRunnerRef],
        now: i64,
    ) -> CoreResult<()> {
        let mut diagnostic = crate::diagnostic_capture::generation_capture(
            self.store.as_ref(),
            Some(&self.guard),
            generation,
            Lane::Readiness,
            QuestionId::Readiness,
            now,
        );
        let Some(identity) = self
            .lifecycle
            .generation_forgejo_runner(&generation.id)
            .await?
        else {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::Quarantined, now)
                .await?;
            return Ok(());
        };
        let mut observed = observe_identity(generation, &identity, runners);
        let detail;
        if matches!(observed, Ok(None)) {
            let Ok(id) = u64::try_from(identity.0) else {
                return Ok(());
            };
            detail = match self.forgejo.get_runner(id).await {
                Ok(detail) => detail,
                Err(_) => {
                    diagnostic.reason(
                        Code::ControlInventoryUnavailable,
                        StageId::RunnerInventory,
                        false,
                    );
                    return Ok(());
                }
            };
            observed = observe_identity(generation, &identity, detail.as_slice());
        }
        let (next, note) = next_state(
            generation.state,
            observed,
            &self.labels,
            generation.created_at,
            now,
        );
        record_note(&mut diagnostic, note);
        if let Some(next) = next {
            self.lifecycle
                .generation_advance(&generation.id, next, now)
                .await?;
        }
        Ok(())
    }

    async fn recover_create(&self, generation: &GenerationRecord, now: i64) -> CoreResult<()> {
        let operations = self
            .lifecycle
            .operations_open_for_generation(&generation.id)
            .await?;
        let identity = self
            .lifecycle
            .generation_forgejo_runner(&generation.id)
            .await?;
        if let Some(operation) = operations
            .iter()
            .find(|op| op.kind == "ForgejoRegistration")
        {
            if identity.is_none() {
                return self
                    .classify_registration(generation, &operation.id, now)
                    .await;
            }
            self.lifecycle
                .operation_update_state(&operation.id, "Completed", now)
                .await?;
        }
        // A stored post-Create result is positive evidence the runtime returned;
        // otherwise cleanup must classify any partial apply from its own ledger.
        let next = if identity.is_some()
            && self
                .lifecycle
                .generation_state_identity(&generation.id)
                .await?
                .is_some()
        {
            for operation in operations.iter().filter(|op| op.kind == "Create") {
                self.lifecycle
                    .operation_update_state(&operation.id, "Completed", now)
                    .await?;
            }
            GenerationState::WaitingOnline
        } else {
            GenerationState::CleanupRequired
        };
        self.lifecycle
            .generation_advance(&generation.id, next, now)
            .await
    }
}
