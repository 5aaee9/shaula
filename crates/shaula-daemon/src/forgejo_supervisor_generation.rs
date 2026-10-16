//! Durable Forgejo Generation creation. Prepare retained inputs before registration.

use shaula_core::error::{CoreError, CoreResult};
use shaula_core::lifecycle::GenerationState;

use super::ForgejoPoolSupervisor;
use crate::pool_create::{GenerationNames, PinnedRevision};

#[path = "forgejo_supervisor_create_stages.rs"]
mod stages;

impl ForgejoPoolSupervisor {
    pub(super) async fn create_one_generation(&self, now: i64) -> CoreResult<bool> {
        use shaula_core::diagnostics::{Code, Lane, QuestionId};
        let _permit = crate::diagnostic_capture::acquire(
            &self.create_limit,
            self.scale_up_observer(),
            Code::ExecutionCreateSlotWait,
            now,
        )
        .await
        .map_err(|_| CoreError::internal("create scheduler stopped"))?;
        let create = self.pool_create();
        // Reserve occupancy under the same exclusive gate used by Fleet writes.
        let gate = self.gates.acquire_exclusive(&self.fleet_key).await;
        let Some((snapshot, spec)) = create.admissible_revision().await? else {
            return Ok(false);
        };
        let pooled = spec.template_pool.is_some() || spec.template_pool_ref.is_some();
        let pinned = PinnedRevision::from_snapshot(pooled, snapshot)?;
        let names =
            GenerationNames::mint(&self.fleet_key, &self.runner_name_prefix, &self.work_root);
        let record = names.record(&self.fleet_key, pinned, now);
        let Some((record, parameters)) = create
            .admit_generation(record, pooled, spec.template_inputs)
            .await?
        else {
            return Ok(false);
        };
        let generation_id = names.generation_id.clone();
        self.lifecycle
            .generation_advance(&generation_id, GenerationState::Creating, now)
            .await?;
        drop(gate);
        let prepared = match create
            .prepare_generation(&record, &parameters, |manifest| {
                manifest.validate_forgejo_targets(&self.bootstrap_labels)
            })
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => return create.cleanup_error(&generation_id, now, error).await,
        };
        let Some(bootstrap) = self
            .register_generation(&record, self.clock.now_unix_ms())
            .await?
        else {
            return Ok(false);
        };
        let (request, apply_intent) = match self
            .create_request(names, prepared, parameters, bootstrap, &record)
            .await
        {
            Ok(built) => built,
            Err(error) => return create.cleanup_error(&generation_id, now, error).await,
        };
        let template_gate = self.template_create.gate(&record.template_profile_key);
        let _template_permit = match &template_gate {
            Some(gate) => Some(
                crate::diagnostic_capture::acquire(
                    gate,
                    self.scale_up_observer(),
                    Code::ExecutionCreateSlotWait,
                    self.clock.now_unix_ms(),
                )
                .await
                .map_err(|_| CoreError::internal("create scheduler stopped"))?,
            ),
            None => None,
        };
        let mut diagnostic = crate::diagnostic_capture::generation_capture(
            self.store.as_ref(),
            Some(&self.guard),
            &record,
            Lane::Operation,
            QuestionId::Readiness,
            now,
        );
        create
            .record_create_outcome(
                &generation_id,
                crate::iac_metrics::timed(
                    crate::iac_metrics::Effect::Create,
                    self.runtime.create(request),
                )
                .await,
                &apply_intent,
                &mut diagnostic,
            )
            .await
    }

    fn scale_up_observer(&self) -> Option<shaula_core::diagnostics::Observer> {
        use shaula_core::diagnostics::{Guard, Lane, Observer, QuestionId};
        Observer::register(
            self.store.diagnostic_sink(),
            Guard::fleet(&self.fleet_key, &self.guard),
            Lane::Operation,
            QuestionId::ScaleUp,
        )
    }
}
