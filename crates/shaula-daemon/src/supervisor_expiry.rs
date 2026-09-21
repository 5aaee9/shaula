//! GitHub hard timeout: destroy first, then retry exact-admission deregistration.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::ports::RemovalOutcome;
use shaula_core::registry::GenerationRecord;

use super::FleetSupervisor;

impl FleetSupervisor {
    pub(super) async fn expire_runners(&self, now: i64) -> CoreResult<u32> {
        let reaper = crate::runner_lifetime::RunnerReaper {
            lifecycle: self.store.as_ref(),
            store: self.handoff.as_ref(),
            runtime: self.runtime.as_ref(),
            clock: self.clock.as_deref(),
            artifact_root: &self.config.artifact_root,
            operation_timeout: self.config.operation_timeout,
            apply_intent: &self.config.apply_intent_sink,
            provider: shaula_core::fleet::FleetProviderKind::Github,
        };
        let mut destroyed = 0;
        for generation in reaper
            .due(&self.config.fleet_key, now, self.runner_max_lifetime)
            .await?
        {
            let _permit =
                self.limits.destroy.acquire().await.map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "destroy scheduler stopped")
                })?;
            if reaper.destroy_resources(&generation, now).await?
                && self.remove_expired_registration(&generation).await?
            {
                reaper.finish(&generation.id, now).await?;
                destroyed += 1;
            }
        }
        Ok(destroyed)
    }

    async fn remove_expired_registration(&self, generation: &GenerationRecord) -> CoreResult<bool> {
        let Some(id) = generation.github_runner_id else {
            // Successful GitHub Creates always retain the JIT runner identity.
            // Absence is NOT proof of deregistration.
            tracing::error!(generation = %generation.id, "expired runner identity missing; cleanup remains pending");
            return Ok(false);
        };
        let Some(reference) = self.handoff.auth_generation_ref(&generation.id).await? else {
            tracing::warn!(generation = %generation.id, "expired runner auth reference missing; cleanup remains pending");
            return Ok(false);
        };
        let current = (
            self.config.auth_profile_key.clone(),
            self.config.auth_revision,
        );
        let github = if reference == current {
            Some(&self.github)
        } else {
            self.revision_clients.get(&reference)
        };
        let Some(github) = github else {
            tracing::warn!(generation = %generation.id, "expired runner auth client unavailable; will retry");
            return Ok(false);
        };
        match github.remove_runner(id).await {
            Ok(RemovalOutcome::Removed | RemovalOutcome::AlreadyAbsent) => Ok(true),
            // A disconnected job can remain busy remotely. Do not pretend the
            // registration disappeared or repeat the completed resource destroy.
            Ok(RemovalOutcome::JobStillRunning) | Err(_) => {
                tracing::warn!(generation = %generation.id, "expired runner deregistration pending; will retry");
                Ok(false)
            }
        }
    }
}
