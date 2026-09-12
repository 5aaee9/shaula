//! Inventory readiness and restart classification; a lost Create is never replayed.

use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::forgejo::ForgejoRunnerRef;
use shaula_core::registry::GenerationRecord;

use super::ForgejoPoolSupervisor;

const READINESS_TIMEOUT_MS: i64 = 15 * 60 * 1000;

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
                    continue;
                }
                GenerationState::Creating => {
                    self.recover_create(&generation, now).await?;
                    continue;
                }
                GenerationState::WaitingOnline | GenerationState::Idle | GenerationState::Busy => {}
                _ => continue,
            }
            let Some(identity) = self
                .lifecycle
                .generation_forgejo_runner(&generation.id)
                .await?
            else {
                self.lifecycle
                    .generation_advance(&generation.id, GenerationState::Quarantined, now)
                    .await?;
                continue;
            };
            let mut observed = observe_identity(&generation, &identity, runners);
            let detail;
            if matches!(observed, Ok(None)) {
                let Ok(id) = u64::try_from(identity.0) else {
                    continue;
                };
                detail = match self.forgejo.get_runner(id).await {
                    Ok(detail) => detail,
                    Err(_) => continue,
                };
                observed = observe_identity(&generation, &identity, detail.as_slice());
            }
            let next = match observed {
                Err(()) => Some(GenerationState::Quarantined),
                Ok(None) => Some(match generation.state {
                    GenerationState::WaitingOnline => GenerationState::CleanupRequired,
                    _ => GenerationState::Retiring,
                }),
                Ok(Some(runner)) => {
                    if !runner.is_known()
                        || (!runner.labels.is_empty()
                            && !super::is_owned_runner(runner, &self.labels))
                    {
                        Some(GenerationState::Quarantined)
                    } else if generation.state == GenerationState::WaitingOnline
                        && runner.is_idle()
                        && super::is_owned_runner(runner, &self.labels)
                    {
                        Some(GenerationState::Idle)
                    } else if generation.state == GenerationState::Idle && runner.is_active() {
                        Some(GenerationState::Busy)
                    } else if generation.state == GenerationState::WaitingOnline
                        && now.saturating_sub(generation.created_at) >= READINESS_TIMEOUT_MS
                        && !runner.is_active()
                    {
                        Some(GenerationState::CleanupRequired)
                    } else {
                        None
                    }
                }
            };
            if let Some(next) = next {
                self.lifecycle
                    .generation_advance(&generation.id, next, now)
                    .await?;
            }
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

/// An ID match with changed UUID/name is NOT absence. A colliding name/UUID is
/// also contradictory evidence, even when the expected ID is missing.
pub(super) fn observe_identity<'a>(
    generation: &GenerationRecord,
    identity: &(i64, String),
    runners: &'a [ForgejoRunnerRef],
) -> Result<Option<&'a ForgejoRunnerRef>, ()> {
    let mut observed = None;
    for runner in runners {
        if i64::try_from(runner.id).ok() == Some(identity.0)
            || runner.uuid == identity.1
            || runner.name == generation.runner_name
        {
            if observed.is_some()
                || i64::try_from(runner.id).ok() != Some(identity.0)
                || runner.uuid != identity.1
                || runner.name != generation.runner_name
                || !runner.ephemeral
            {
                return Err(());
            }
            observed = Some(runner);
        }
    }
    Ok(observed)
}
