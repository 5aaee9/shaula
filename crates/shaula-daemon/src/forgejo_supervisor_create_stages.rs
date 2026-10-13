//! The Forgejo-specific create stage: the protected bootstrap request. The
//! backend-neutral stages live in [`crate::pool_create`].

use std::sync::Arc;

use serde_json::{Map, Value};
use shaula_core::error::CoreResult;
use shaula_core::fleet::FleetProviderKind;
use shaula_core::ports::forgejo::ForgejoBootstrapMaterial;
use shaula_core::ports::TemplateCreateRequest;
use shaula_core::registry::GenerationRecord;
use shaula_core::template::{
    BindingsDigest, ForgejoVmBootstrap, GenerationIdentity, SetupInfoDescriptor,
    ShaulaInputEnvelope,
};

use super::ForgejoPoolSupervisor;
use crate::apply_intent::TrackedApplyIntentSink;
use crate::pool_create::{GenerationNames, PoolCreate, PreparedGeneration};

impl ForgejoPoolSupervisor {
    pub(super) fn pool_create(&self) -> PoolCreate<'_> {
        PoolCreate {
            fleet_key: &self.fleet_key,
            guard: &self.guard,
            auth: &self.auth,
            capacity: self.capacity,
            provider: FleetProviderKind::Forgejo,
            store: self.store.as_ref(),
            lifecycle: self.lifecycle.as_ref(),
            runtime: self.runtime.as_ref(),
            clock: self.clock.as_ref(),
            artifact_root: &self.artifact_root,
            operation_timeout: self.operation_timeout,
        }
    }

    /// The protected envelope plus the create request carrying it. Errors
    /// come only from setup-info attachment or manifest validation.
    pub(super) async fn create_request(
        &self,
        names: GenerationNames,
        prepared: PreparedGeneration,
        parameters: Map<String, Value>,
        bootstrap: ForgejoBootstrapMaterial,
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
        input.forgejo = Some(bootstrap.identity());
        if manifest.forgejo_vm_bootstrap_contract.is_some() {
            input.forgejo_vm = Some(ForgejoVmBootstrap::from_registration(&bootstrap));
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
            forgejo_bootstrap: Some(bootstrap),
            gitlab_bootstrap: None,
        };
        Ok((request, apply_intent))
    }
}
