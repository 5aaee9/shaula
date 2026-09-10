//! Persist JIT intent and its definite outcome without releasing uncertainty.

use super::FleetSupervisor;
use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::{EffectOutcome, JitConfig};

impl FleetSupervisor {
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
            Ok(EffectOutcome::Definite(jit)) => jit,
            Ok(EffectOutcome::Uncertain { summary, .. }) => {
                tracing::warn!(
                    generation = %generation_id,
                    summary = %summary,
                    "jit mint uncertain; generation quarantined"
                );
                self.store
                    .generation_advance(generation_id, GenerationState::Quarantined, now)
                    .await?;
                return Ok(None);
            }
            Err(failure) => {
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
}
