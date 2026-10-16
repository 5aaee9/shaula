//! Durable GitLab Generation creation on the shared Pool create stages.
//! Prepare retained inputs before registration; the token reaches the
//! runtime only as protected bootstrap material, never the envelope.

use std::sync::Arc;

use serde_json::{Map, Value};
use shaula_core::error::{CoreError, CoreResult};
use shaula_core::fleet::FleetProviderKind;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::gitlab::GitlabBootstrapMaterial;
use shaula_core::ports::TemplateCreateRequest;
use shaula_core::registry::GenerationRecord;
use shaula_core::template::{
    BindingsDigest, GenerationIdentity, GitlabVmBootstrap, SetupInfoDescriptor, ShaulaInputEnvelope,
};

use super::GitlabPoolSupervisor;
use crate::apply_intent::TrackedApplyIntentSink;
use crate::pool_create::{GenerationNames, PinnedRevision, PoolCreate, PreparedGeneration};

impl GitlabPoolSupervisor {
    fn pool_create(&self) -> PoolCreate<'_> {
        PoolCreate {
            fleet_key: &self.fleet_key,
            guard: &self.guard,
            auth: &self.auth,
            capacity: self.capacity,
            provider: FleetProviderKind::Gitlab,
            store: self.store.as_ref(),
            lifecycle: self.lifecycle.as_ref(),
            runtime: self.runtime.as_ref(),
            clock: self.clock.as_ref(),
            artifact_root: &self.artifact_root,
            operation_timeout: self.operation_timeout,
        }
    }

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
                manifest.validate_gitlab_generation_target()
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

    async fn create_request(
        &self,
        names: GenerationNames,
        prepared: PreparedGeneration,
        parameters: Map<String, Value>,
        bootstrap: GitlabBootstrapMaterial,
        record: &GenerationRecord,
    ) -> CoreResult<(TemplateCreateRequest, Arc<TrackedApplyIntentSink>)> {
        let PreparedGeneration {
            artifact_dir,
            manifest,
            bindings,
            bindings_digest,
        } = prepared;
        let mut input = ShaulaInputEnvelope::new(
            GenerationIdentity {
                fleet_key: self.fleet_key.clone(),
                scale_set_id: None,
                id: names.generation_id,
                runner_name: names.runner_name,
                generation_name: names.generation_name,
            },
            String::new(),
            BindingsDigest(bindings_digest.clone()),
        );
        input.bindings = bindings;
        input.gitlab = Some(bootstrap.identity());
        if manifest.gitlab_vm_bootstrap_contract.is_some() {
            input.gitlab_vm = Some(GitlabVmBootstrap::from_registration(&bootstrap));
        }
        input.parameters = parameters;
        if manifest.input_contract_version == 2 {
            let descriptor = match self.setup_info_issuer.as_deref() {
                Some(issuer) => issuer
                    .issue(
                        &input.generation.id,
                        self.clock.now_unix_ms().div_euclid(1000),
                    )
                    .await
                    .unwrap_or(SetupInfoDescriptor::Disabled),
                None => SetupInfoDescriptor::Disabled,
            };
            input = input.with_setup_info(descriptor)?;
        }
        input.validate_for_manifest(&manifest)?;
        let apply_intent = TrackedApplyIntentSink::new(self.apply_intent_sink.clone());
        let request = TemplateCreateRequest {
            workspace_path: names.workspace,
            artifact_dir,
            pinned_artifact_digest: record.template_artifact_digest.clone(),
            input,
            expected_bindings_digest: BindingsDigest(bindings_digest),
            managed_shape: manifest.managed_resource_shape,
            environment: Vec::new(),
            timeout: self.operation_timeout,
            apply_intent_sink: Some(apply_intent.clone()),
            forgejo_bootstrap: None,
            gitlab_bootstrap: Some(bootstrap),
        };
        Ok((request, apply_intent))
    }
}
