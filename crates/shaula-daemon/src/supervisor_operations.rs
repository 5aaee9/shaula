//! Persist JIT intent and its definite outcome without releasing uncertainty.

use super::FleetSupervisor;
use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::{EffectOutcome, JitConfig};
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

/// One bounded GitHub runner operation completed with the given outcome.
fn record_runner(result: MetricResult) {
    TelemetryHandle::new().record(MetricOperation::Runner, result, 1);
}

impl FleetSupervisor {
    #[tracing::instrument(name = "shaula.github.jit_mint", skip_all, fields(fleet_key = %self.config.fleet_key, generation_id = %generation_id, scale_set_id = scale_set_id))]
    pub(super) async fn acquire_generation_jit(
        &self,
        generation_id: &str,
        scale_set_id: i64,
        runner_name: &str,
        context: serde_json::Value,
        attempt_id: String,
        now: i64,
    ) -> CoreResult<Option<JitConfig>> {
        let request_digest = format!("sha256:{}", {
            use sha2::Digest as _;
            hex::encode(sha2::Sha256::digest(context.to_string().as_bytes()))
        });
        let operation_id = format!("jit-{generation_id}");
        self.store
            .generation_set_jit_phase(generation_id, "JITStarting", now)
            .await?;
        self.store
            .operation_insert(shaula_core::registry::OperationInsert {
                id: operation_id.clone(),
                generation_id: generation_id.to_string(),
                kind: "JitStarting".to_string(),
                state: "Pending".to_string(),
                provenance_json: Some(
                    serde_json::json!({
                        "context": context,
                        "request_digest": request_digest,
                        "jit_attempt_id": attempt_id,
                    })
                    .to_string(),
                ),
                saved_plan_path: None,
                saved_plan_digest: None,
                now,
            })
            .await?;
        let jit = match self.github.generate_jit(scale_set_id, runner_name).await {
            Ok(EffectOutcome::Definite(jit)) => {
                record_runner(MetricResult::Ok);
                jit
            }
            Ok(EffectOutcome::Uncertain { summary }) => {
                record_runner(MetricResult::Degraded);
                tracing::warn!(
                    generation = %generation_id,
                    summary = %summary,
                    "jit mint uncertain; classifying by exact-name lookup (spec 0025)"
                );
                return self
                    .recover_uncertain_jit(generation_id, scale_set_id, runner_name, now)
                    .await;
            }
            Err(failure) => {
                record_runner(MetricResult::Failed);
                tracing::warn!(
                    generation = %generation_id,
                    summary = %failure.summary(),
                    "jit mint failed; generation quarantined"
                );
                self.store
                    .generation_advance(generation_id, GenerationState::Quarantined, now)
                    .await?;
                return Ok(None);
            }
        };
        // Record recovery evidence before completing the intent. If storage
        // fails at any step, its open row still retains the exact auth ref.
        self.store
            .generation_set_jit_phase(generation_id, "JITAcquired", now)
            .await?;
        self.store
            .generation_set_github_runner(generation_id, jit.runner.id, now)
            .await?;
        self.store
            .operation_update_state(&operation_id, "Succeeded", now)
            .await?;
        Ok(Some(jit))
    }

    /// Spec 0025: an Uncertain mint is classified by exact-name lookup.
    /// A landed entity is recorded and removed (no orphan inventory); a
    /// proven-absent mint routes the resource-free generation to cleanup;
    /// ambiguity and lookup/removal failures stay quarantined.
    async fn recover_uncertain_jit(
        &self,
        generation_id: &str,
        scale_set_id: i64,
        runner_name: &str,
        now: i64,
    ) -> CoreResult<Option<JitConfig>> {
        use shaula_core::ports::{RemovalOutcome, RunnerLookup};
        match self
            .github
            .get_runner_by_name(scale_set_id, runner_name)
            .await
        {
            Ok(RunnerLookup::ExactlyOne(runner)) => {
                // The encoded JIT config is unrecoverable; the generation
                // can never boot. Record the identity so inventory stays
                // attributable, then converge the entity away.
                self.store
                    .generation_set_github_runner(generation_id, runner.id, now)
                    .await?;
                match self.github.remove_runner(runner.id).await {
                    Ok(RemovalOutcome::Removed) | Ok(RemovalOutcome::AlreadyAbsent) => {
                        tracing::warn!(
                            generation = %generation_id,
                            runner = runner.id,
                            "uncertain mint landed; entity removed, generation to cleanup"
                        );
                        self.store
                            .generation_advance(
                                generation_id,
                                GenerationState::CleanupRequired,
                                now,
                            )
                            .await?;
                    }
                    outcome => {
                        let summary = match outcome {
                            Ok(RemovalOutcome::JobStillRunning) => "runner still has a job",
                            _ => "removal unavailable",
                        };
                        tracing::warn!(
                            generation = %generation_id,
                            runner = runner.id,
                            summary,
                            "uncertain mint landed but removal blocked; generation quarantined"
                        );
                        self.store
                            .generation_advance(generation_id, GenerationState::Quarantined, now)
                            .await?;
                    }
                }
            }
            Ok(RunnerLookup::None) => {
                tracing::warn!(
                    generation = %generation_id,
                    "uncertain mint proven absent; generation to cleanup"
                );
                self.store
                    .generation_advance(generation_id, GenerationState::CleanupRequired, now)
                    .await?;
            }
            Ok(RunnerLookup::Multiple) => {
                tracing::warn!(
                    generation = %generation_id,
                    "ambiguous exact-name lookup; generation quarantined"
                );
                self.store
                    .generation_advance(generation_id, GenerationState::Quarantined, now)
                    .await?;
            }
            Err(failure) => {
                tracing::warn!(
                    generation = %generation_id,
                    summary = %failure.summary(),
                    "classification lookup failed; generation quarantined"
                );
                self.store
                    .generation_advance(generation_id, GenerationState::Quarantined, now)
                    .await?;
            }
        }
        Ok(None)
    }
}
