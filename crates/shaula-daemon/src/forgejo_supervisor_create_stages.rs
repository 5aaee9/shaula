//! Named stages of the Forgejo generation create: the gated admission
//! read, the frozen pin and identity, the durable row, the protected
//! request and the recorded apply outcome.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use shaula_core::diagnostics::{Capture, Code, Outcome, StageId};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::{FleetProviderKind, FleetSpec};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::forgejo::ForgejoBootstrapMaterial;
use shaula_core::ports::{TemplateCreateRequest, TemplateCreateResult, TemplateOutcomeError};
use shaula_core::registry::{FleetRevisionRow, GenerationRecord};
use shaula_core::template::{
    BindingsDigest, ForgejoVmBootstrap, GenerationIdentity, ProfileManifest, SetupInfoDescriptor,
    ShaulaInputEnvelope,
};
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use super::ForgejoPoolSupervisor;
use crate::apply_intent::TrackedApplyIntentSink;

/// The admitted revision fields frozen into the generation record.
pub(super) struct PinnedRevision {
    profile_key: String,
    template_revision: i64,
    artifact_digest: String,
    attestation_id: String,
    fleet_revision: i64,
    inputs_digest: String,
}

impl PinnedRevision {
    /// Pool placeholders never reach storage: the common admission
    /// transaction selects and freezes the exact route before inserting
    /// the Generation.
    pub(super) fn from_snapshot(pooled: bool, snapshot: FleetRevisionRow) -> CoreResult<Self> {
        let (profile_key, template_revision, artifact_digest, attestation_id) = if pooled {
            (String::new(), 0, String::new(), String::new())
        } else {
            match (
                snapshot.template_profile_key,
                snapshot.template_revision,
                snapshot.template_artifact_digest,
                snapshot.template_attestation_id,
            ) {
                (Some(key), Some(revision), Some(digest), Some(attestation)) => {
                    (key, revision, digest, attestation)
                }
                _ => {
                    return Err(CoreError::new(
                        ReasonCode::TemplateInvalid,
                        "Forgejo revision has no resolved template pin",
                    ))
                }
            }
        };
        Ok(Self {
            profile_key,
            template_revision,
            artifact_digest,
            attestation_id,
            fleet_revision: snapshot.revision,
            inputs_digest: snapshot.inputs_digest,
        })
    }
}

/// Durable identity of one new Forgejo generation.
pub(super) struct GenerationNames {
    pub(super) generation_id: String,
    runner_name: String,
    generation_name: String,
    workspace: PathBuf,
}

impl GenerationNames {
    pub(super) fn mint(supervisor: &ForgejoPoolSupervisor) -> Self {
        let generation_id = shaula_core::auth::new_attempt_id();
        let generation_name = crate::runner_operation::generation_resource_name(&generation_id);
        let runner_name = format!("{}{}", supervisor.runner_name_prefix, generation_id);
        let workspace = supervisor
            .work_root
            .join(&supervisor.fleet_key)
            .join(&generation_id);
        Self {
            generation_id,
            runner_name,
            generation_name,
            workspace,
        }
    }

    pub(super) fn record(
        &self,
        fleet_key: &str,
        pinned: PinnedRevision,
        now: i64,
    ) -> GenerationRecord {
        GenerationRecord {
            id: self.generation_id.clone(),
            fleet_key: fleet_key.to_owned(),
            runner_name: self.runner_name.clone(),
            generation_name: self.generation_name.clone(),
            fleet_revision: pinned.fleet_revision,
            template_profile_key: pinned.profile_key,
            pool_member_key: None,
            template_revision: pinned.template_revision,
            template_artifact_digest: pinned.artifact_digest,
            attestation_id: pinned.attestation_id,
            inputs_digest: pinned.inputs_digest,
            state: GenerationState::CreatePending,
            github_runner_id: None,
            workspace_path: self.workspace.to_string_lossy().into_owned(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Retained local material from `prepare_generation`.
pub(super) struct PreparedGeneration {
    pub(super) artifact_dir: PathBuf,
    pub(super) manifest: ProfileManifest,
    pub(super) bindings: Map<String, Value>,
    pub(super) bindings_digest: String,
}

impl ForgejoPoolSupervisor {
    /// Admission reads under the exclusive Fleet gate. Never borrow a newer
    /// revision with a client built for an older one; `None` skips this
    /// create without any durable effect.
    pub(super) async fn admissible_revision(
        &self,
    ) -> CoreResult<Option<(FleetRevisionRow, FleetSpec)>> {
        if self
            .current_head()
            .await?
            .is_none_or(|head| head.deletion_marker)
        {
            return Ok(None);
        }
        if self.store.generations_occupancy(&self.fleet_key).await? >= self.capacity.max_runners {
            return Ok(None);
        }
        let Some(snapshot) = self.store.fleet_revision_latest(&self.fleet_key).await? else {
            return Ok(None);
        };
        if snapshot.revision != self.guard.desired_revision || snapshot.auth_desired != self.auth {
            return Ok(None);
        }
        let spec: FleetSpec = serde_json::from_str(&snapshot.spec_json)
            .map_err(|_| CoreError::new(ReasonCode::SpecInvalid, "stored Fleet spec invalid"))?;
        if spec.kind != FleetProviderKind::Forgejo {
            return Ok(None);
        }
        Ok(Some((snapshot, spec)))
    }

    /// Inserts the durable generation row (pool admission or guarded
    /// insert); returns the stored record and its envelope parameters.
    pub(super) async fn admit_generation(
        &self,
        record: GenerationRecord,
        pooled: bool,
        parameters: Map<String, Value>,
    ) -> CoreResult<Option<(GenerationRecord, Map<String, Value>)>> {
        if pooled {
            let Some(admission) = self
                .lifecycle
                .generation_admit_pool(record, &self.guard)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((admission.generation, admission.template_inputs)));
        }
        if !self
            .lifecycle
            .generation_insert_guarded(record.clone(), &self.guard)
            .await?
        {
            return Ok(None);
        }
        Ok(Some((record, parameters)))
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
        };
        Ok((request, apply_intent))
    }

    /// Records the apply outcome: result + WaitingOnline on success,
    /// CleanupRequired on failure.
    pub(super) async fn record_create_outcome(
        &self,
        generation_id: &str,
        result: Result<TemplateCreateResult, TemplateOutcomeError>,
        apply_intent: &TrackedApplyIntentSink,
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
                TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Ok, 1);
                let result_json = serde_json::json!({
                    "result": result.result_envelope,
                    "state_lineage": result.state_lineage,
                    "state_serial": result.state_serial,
                })
                .to_string();
                let digest = format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(result_json.as_bytes()))
                );
                let now = self.clock.now_unix_ms();
                self.lifecycle
                    .generation_set_result(generation_id, &result_json, &digest, now)
                    .await?;
                apply_intent.complete(self.lifecycle.as_ref(), now).await?;
                self.lifecycle
                    .generation_advance(generation_id, GenerationState::WaitingOnline, now)
                    .await?;
                Ok(true)
            }
            Err(error) => {
                let known = matches!(error, TemplateOutcomeError::PlanFailed { .. });
                diagnostic.reason(
                    if known {
                        Code::LifecycleOperationFailed
                    } else {
                        Code::LifecycleApplyOutcomeUnknown
                    },
                    StageId::CreateEffect,
                    known,
                );
                TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Failed, 1);
                self.lifecycle
                    .generation_advance(
                        generation_id,
                        GenerationState::CleanupRequired,
                        self.clock.now_unix_ms(),
                    )
                    .await?;
                Ok(false)
            }
        }
    }
}
