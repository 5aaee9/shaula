//! Classifies the actual runtime result and commits the existing lifecycle facts.
use super::{record_iac, FleetSupervisor};
use shaula_core::{
    diagnostics::*,
    error::CoreResult,
    ports::{TemplateCreateResult, TemplateOutcomeError},
};
use shaula_observability::MetricResult;
impl FleetSupervisor {
    pub(super) async fn finish_generation_create(
        &self,
        generation_id: &str,
        result: Result<TemplateCreateResult, TemplateOutcomeError>,
        apply_intent: &crate::apply_intent::TrackedApplyIntentSink,
        now: i64,
        diagnostic: &mut Capture,
    ) -> CoreResult<bool> {
        match result {
            Ok(result) => {
                diagnostic.pass(StageId::CreateEffect);
                diagnostic.reason(
                    Code::LifecycleAwaitingOnline,
                    StageId::RunnerInventory,
                    false,
                );
                diagnostic.outcome(Outcome::Progressing);
                // Start the hard lifetime at successful Create, not at tick/allocation.
                let now = self.clock.as_ref().map_or(now, |clock| clock.now_unix_ms());
                record_iac(MetricResult::Ok);
                // Persist the result envelope TOGETHER WITH the REAL
                // post-apply state identity: this is the ownership proof
                // every later Destroy re-verifies at its effect boundary
                // (F07, spec 0004 §5).
                let body = serde_json::json!({
                    "result": result.result_envelope,
                    "state_lineage": result.state_lineage,
                    "state_serial": result.state_serial,
                })
                .to_string();
                self.store
                    .generation_set_result(generation_id, &body, "sha256:result", now)
                    .await?;
                apply_intent.complete(self.store.as_ref(), now).await?;
                self.store
                    .generation_advance(
                        generation_id,
                        shaula_core::lifecycle::GenerationState::WaitingOnline,
                        now,
                    )
                    .await?;
                Ok(true)
            }
            Err(shaula_core::ports::TemplateOutcomeError::PlanFailed { .. }) => {
                diagnostic.reason(Code::LifecycleOperationFailed, StageId::CreateEffect, true);
                record_iac(MetricResult::Failed);
                // No apply admitted; the generation never touched infra.
                self.store
                    .generation_advance(
                        generation_id,
                        shaula_core::lifecycle::GenerationState::CleanupRequired,
                        now,
                    )
                    .await?;
                Ok(false)
            }
            Err(_) => {
                diagnostic.reason(
                    Code::LifecycleApplyOutcomeUnknown,
                    StageId::CreateEffect,
                    true,
                );
                record_iac(MetricResult::Failed);
                // Apply may have started: never re-apply; cleanup path.
                self.store
                    .generation_advance(
                        generation_id,
                        shaula_core::lifecycle::GenerationState::CleanupRequired,
                        now,
                    )
                    .await?;
                Ok(false)
            }
        }
    }
}
