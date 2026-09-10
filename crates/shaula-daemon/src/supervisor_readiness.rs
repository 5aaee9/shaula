//! Generation readiness reconciliation (spec 0024): drives the two
//! `WaitingOnline` exits designed in spec 0001's state machine —
//! inventory-online advances to `Idle`, and the readiness timeout (which
//! uniformly covers a runner that never registered AND an ephemeral JIT
//! runner that served its single job and self-deregistered) moves the
//! generation into `CleanupRequired` and the existing destroy channel.

use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState;

use super::FleetSupervisor;

/// Grace before a not-yet-observed `WaitingOnline` generation is sent to
/// cleanup: covers VM clone, boot, cloud-init and runner registration.
pub const READINESS_TIMEOUT_MS: i64 = 15 * 60 * 1000;

impl FleetSupervisor {
    /// Runs between the listener gate and capacity convergence so a
    /// transition lands in this tick's own retire/counter pass. A failed
    /// inventory read only WARNs: the reconciliation is level-triggered
    /// and retries on the next tick.
    pub(super) async fn reconcile_generation_readiness(&self, now: i64) -> CoreResult<()> {
        let waiting: Vec<_> = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?
            .into_iter()
            .filter(|generation| generation.state == GenerationState::WaitingOnline)
            .collect();
        if waiting.is_empty() {
            // Zero extra API traffic in the steady state (spec 0024 §1).
            return Ok(());
        }
        let Some(scale_set_id) = self
            .store
            .scale_set_get(&self.config.fleet_key)
            .await?
            .and_then(|s| s.scale_set_id)
        else {
            return Ok(());
        };
        let runners = match self.github.list_runners(scale_set_id).await {
            Ok(runners) => runners,
            Err(failure) => {
                tracing::warn!(fleet = %self.config.fleet_key, summary = %failure.summary(), "readiness inventory unavailable");
                return Ok(());
            }
        };
        for generation in waiting {
            let online = runners.iter().any(|runner| {
                runner.scale_set_id == scale_set_id
                    && generation.github_runner_id == Some(runner.id)
                    && generation.runner_name == runner.name
                    && runner.status.eq_ignore_ascii_case("online")
            });
            if online {
                // Idle joins the retirement channel (retire_excess's
                // needs_removal set) in this same tick.
                if let Err(e) = self
                    .store
                    .generation_advance(&generation.id, GenerationState::Idle, now)
                    .await
                {
                    tracing::debug!(generation = %generation.id, summary = %e, "readiness idle transition rejected");
                }
                continue;
            }
            let age = now.saturating_sub(generation.created_at);
            if age > READINESS_TIMEOUT_MS {
                // Covers both "never registered" and "ephemeral runner
                // served its job and self-deregistered": the inventory
                // cannot distinguish them, and both must converge to
                // terraform destroy (spec 0024 §2).
                if let Err(e) = self
                    .store
                    .generation_advance(&generation.id, GenerationState::CleanupRequired, now)
                    .await
                {
                    tracing::debug!(generation = %generation.id, summary = %e, "readiness cleanup transition rejected");
                }
            }
            // Within the grace period (or offline mid-boot): keep waiting.
        }
        Ok(())
    }
}
