//! Generation readiness reconciliation (spec 0024): drives the
//! `WaitingOnline` exits designed in spec 0001's state machine —
//! inventory-online advances to `Idle`, and the readiness timeout (which
//! uniformly covers a runner that never registered AND an ephemeral JIT
//! runner that served its single job and self-deregistered) moves the
//! generation into `CleanupRequired` and the existing destroy channel.
//!
//! Rev 3 additionally reconciles `Idle` generations against inventory:
//! a JIT ephemeral runner that was observed online and then served its
//! single job self-deregisters and vanishes from inventory. Without a
//! re-check, such a phantom `Idle` generation holds effective capacity
//! forever — the fleet reports demand but never creates a replacement
//! and never destroys the dead resource (2026-09-12 incident). `Idle`
//! plus a missing or non-online inventory entry advances to `Retiring`,
//! entering the normal remove-then-destroy chain.

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
        let observed: Vec<_> = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?
            .into_iter()
            .filter(|generation| {
                matches!(
                    generation.state,
                    GenerationState::WaitingOnline | GenerationState::Idle
                )
            })
            .collect();
        if observed.is_empty() {
            // Zero extra API traffic in the steady state (spec 0024 §1):
            // no WaitingOnline or Idle generation means nothing to
            // reconcile against the inventory.
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
        for generation in observed {
            let online = runners.iter().any(|runner| {
                runner.scale_set_id == scale_set_id
                    && generation.github_runner_id == Some(runner.id)
                    && generation.runner_name == runner.name
                    && runner.status.eq_ignore_ascii_case("online")
            });
            if generation.state == GenerationState::Idle {
                if online {
                    continue;
                }
                // A previously-online Idle generation whose runner is
                // absent or no longer online can never accept another
                // job: the JIT ephemeral runner either served its single
                // job and self-deregistered (absent) or the agent died
                // without restarting (offline; the runner unit is
                // Restart=no). Retirement removes the entity when it
                // still exists (JobStillRunning re-gates) and destroys
                // the dead infrastructure — it never strands capacity
                // on a phantom runner (spec 0024 rev 3).
                if let Err(e) = self
                    .store
                    .generation_advance(&generation.id, GenerationState::Retiring, now)
                    .await
                {
                    tracing::debug!(generation = %generation.id, summary = %e, "readiness retiring transition rejected");
                }
                continue;
            }
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
