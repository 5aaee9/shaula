//! Runner generation lifecycle: create/retire/destroy flows as inherent
//! methods on [`FleetSupervisor`]. The submodule keeps the host file within
//! the 400-line limit while methods keep private-field access.

use shaula_core::error::{CoreError, CoreResult};
use shaula_core::lifecycle::GenerationState;
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use super::{fingerprint, FleetSupervisor};
use create_stages::{resolve_pin, uses_template_pool, GenerationNames};

/// One bounded IaC engine operation completed with the given outcome.
fn record_iac(result: MetricResult) {
    TelemetryHandle::new().record(MetricOperation::IaC, result, 1);
}

#[path = "supervisor_operations.rs"]
mod operations;
#[path = "supervisor_setup_info.rs"]
mod setup_info;
#[path = "supervisor_template.rs"]
mod template;

impl FleetSupervisor {
    /// Creates one runner generation: durable identity, then JIT (once), then
    /// apply (once) then WaitingOnline. The Create claim re-validates the
    /// fleet deletion marker before any external effect (0002 section 8).
    #[tracing::instrument(name = "shaula.iac.generation_create", skip_all, fields(fleet_key = %self.config.fleet_key))]
    pub(crate) async fn create_one_generation(&self, now: i64) -> CoreResult<bool> {
        use shaula_core::diagnostics::{Code, Lane, QuestionId};
        let _permit = crate::diagnostic_capture::acquire(
            &self.limits.create,
            self.scale_up_observer(),
            Code::ExecutionCreateSlotWait,
            now,
        )
        .await
        .map_err(|_| CoreError::internal("create scheduler stopped"))?;
        if !self.route_proof_admits().await {
            return Ok(false);
        }
        let Some((head, snapshot)) = self.admitted_snapshot().await? else {
            return Ok(false);
        };
        let spec: shaula_core::fleet::FleetSpec = serde_json::from_str(&snapshot.spec_json)
            .map_err(|e| CoreError::internal(format!("spec invalid: {e}")))?;
        let pooled = uses_template_pool(&spec);
        let pin = resolve_pin(pooled, &snapshot)?;
        let names = GenerationNames::mint(&self.config.fleet_key, &self.config.work_root)?;
        let record = names.record(&self.config.fleet_key, &snapshot, pin, now);
        let Some((record, parameters)) = self
            .admit_generation(record, &head, pooled, spec.template_inputs)
            .await?
        else {
            return Ok(false);
        };
        if record.state == GenerationState::CreatePending {
            self.store
                .generation_advance(&names.generation_id, GenerationState::Creating, now)
                .await?;
        }
        let local: CoreResult<Option<_>> = async {
            let scale_set_id = self.bound_scale_set_id().await?;
            if !self
                .fence_holds(&names.generation_id, snapshot.revision, now)
                .await?
            {
                return Ok(None);
            }
            let artifact_dir = self.prepare_workspace(&record, &names.workspace).await?;
            // The backend selection and images belong to the exact protected
            // Template Revision. Check them before minting any remote JIT.
            let material = self
                .template_material(&artifact_dir, &record, &parameters)
                .await?;
            Ok(Some((scale_set_id, artifact_dir, material)))
        }
        .await;
        let (scale_set_id, artifact_dir, material) = match local {
            Ok(Some(prepared)) => prepared,
            Ok(None) => return Ok(false),
            Err(error) => {
                self.abandon_before_jit(&names.generation_id, now).await;
                return Err(error);
            }
        };
        let Some(jit) = self
            .mint_generation_jit(&names, &record, scale_set_id, now)
            .await?
        else {
            return Ok(false);
        };
        let generation_id = names.generation_id.clone();
        let (request, apply_intent) = self
            .create_request(
                create_stages::CreateInputs {
                    names,
                    scale_set_id,
                    jit,
                    material,
                    parameters,
                    artifact_dir,
                },
                &record,
            )
            .await?;
        let template_gate = self
            .limits
            .template_create
            .gate(&record.template_profile_key);
        let _template_permit = match &template_gate {
            Some(gate) => Some(
                crate::diagnostic_capture::acquire(
                    gate,
                    self.scale_up_observer(),
                    Code::ExecutionCreateSlotWait,
                    self.clock.as_ref().map_or(now, |clock| clock.now_unix_ms()),
                )
                .await
                .map_err(|_| CoreError::internal("create scheduler stopped"))?,
            ),
            None => None,
        };
        let mut diagnostic = crate::diagnostic_capture::generation_capture(
            self.handoff.as_ref(),
            self.runtime_guard.as_ref(),
            &record,
            Lane::Operation,
            QuestionId::Readiness,
            now,
        );
        self.finish_generation_create(
            &generation_id,
            self.runtime.create(request).await,
            &apply_intent,
            now,
            &mut diagnostic,
        )
        .await
    }

    /// A failure before JIT has crossed no remote effect, but a generation
    /// left in `Creating` still counts as occupancy and nothing resumes it
    /// until restart recovery. Hand it to the Destroy chain, which settles a
    /// never-started Create without touching any runner resource.
    async fn abandon_before_jit(&self, generation_id: &str, now: i64) {
        if let Err(error) = self
            .store
            .generation_advance(generation_id, GenerationState::CleanupRequired, now)
            .await
        {
            tracing::warn!(
                generation = %generation_id,
                code = error.code.as_str(),
                "failed pre-JIT create not handed to cleanup"
            );
        }
    }

    fn scale_up_observer(&self) -> Option<shaula_core::diagnostics::Observer> {
        use shaula_core::diagnostics::{Guard, Lane, Observer, QuestionId};
        let guard = self.runtime_guard.as_ref()?;
        Observer::register(
            self.handoff.diagnostic_sink(),
            Guard::fleet(&self.config.fleet_key, guard),
            Lane::Operation,
            QuestionId::ScaleUp,
        )
    }
}
#[path = "supervisor_create_result.rs"]
mod create_result;
#[path = "supervisor_create_stages.rs"]
mod create_stages;
