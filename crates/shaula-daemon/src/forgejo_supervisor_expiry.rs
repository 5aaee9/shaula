//! Hard lifetime is not proactive idle drain: terminating active jobs is intentional.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::forgejo::ForgejoRemovalOutcome;
use shaula_core::registry::GenerationRecord;

use super::{readiness::observe_identity, ForgejoPoolSupervisor};

impl ForgejoPoolSupervisor {
    pub(super) async fn expire_runners(&self, now: i64) -> CoreResult<u32> {
        let reaper = crate::runner_lifetime::RunnerReaper {
            lifecycle: self.lifecycle.as_ref(),
            store: self.store.as_ref(),
            runtime: self.runtime.as_ref(),
            clock: Some(self.clock.as_ref()),
            artifact_root: &self.artifact_root,
            operation_timeout: self.operation_timeout,
            apply_intent: &self.apply_intent_sink,
            provider: shaula_core::fleet::FleetProviderKind::Forgejo,
        };
        let mut destroyed = 0;
        for generation in reaper
            .due(&self.fleet_key, now, self.runner_max_lifetime)
            .await?
        {
            let _permit =
                self.destroy_limit.acquire().await.map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "destroy scheduler stopped")
                })?;
            if reaper.destroy_resources(&generation, now).await?
                && self.remove_expired_registration(&generation, now).await?
            {
                reaper.finish(&generation.id, now).await?;
                destroyed += 1;
            }
        }
        Ok(destroyed)
    }

    async fn remove_expired_registration(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<bool> {
        let _claim = self.gates.acquire_claim(&self.fleet_key).await;
        if self.current_head().await?.is_none() {
            return Ok(false);
        }
        let Some(identity) = self
            .lifecycle
            .generation_forgejo_runner(&generation.id)
            .await?
        else {
            tracing::error!(generation = %generation.id, "expired runner identity missing; cleanup remains pending");
            return Ok(false);
        };
        let Ok(id) = u64::try_from(identity.0) else {
            return Ok(false);
        };
        let detail = match self.forgejo.get_runner(id).await {
            Ok(detail) => detail,
            Err(_) => return Ok(false),
        };
        // Use exact ID + UUID + name + ephemeral proof, never a name-only delete.
        match observe_identity(generation, &identity, detail.as_slice()) {
            Ok(None) if detail.is_none() => return Ok(true),
            Ok(Some(_)) => {}
            Err(()) | Ok(None) => {
                self.lifecycle
                    .generation_advance(&generation.id, GenerationState::Quarantined, now)
                    .await?;
                return Ok(false);
            }
        }
        match self.forgejo.delete_runner(id).await {
            Ok(ForgejoRemovalOutcome::Removed | ForgejoRemovalOutcome::AlreadyAbsent) => Ok(true),
            _ => {
                tracing::warn!(generation = %generation.id, "expired runner deregistration pending; will retry");
                Ok(false)
            }
        }
    }
}
