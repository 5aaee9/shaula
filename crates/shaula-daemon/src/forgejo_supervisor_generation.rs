//! Durable Forgejo Generation creation. Prepare retained inputs before registration.

use sha2::{Digest, Sha256};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetProviderKind;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::TemplateCreateRequest;
use shaula_core::registry::GenerationRecord;
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use super::ForgejoPoolSupervisor;

impl ForgejoPoolSupervisor {
    pub(super) async fn create_one_generation(&self, now: i64) -> CoreResult<bool> {
        let _permit = self
            .create_limit
            .acquire()
            .await
            .map_err(|_| CoreError::new(ReasonCode::Internal, "create scheduler stopped"))?;
        // Reserve occupancy under the same exclusive gate used by Fleet writes.
        // Never borrow a newer revision with a client built for an older one.
        let gate = self.gates.acquire_exclusive(&self.fleet_key).await;
        if self
            .current_head()
            .await?
            .is_none_or(|head| head.deletion_marker)
        {
            return Ok(false);
        }
        if self.store.generations_occupancy(&self.fleet_key).await? >= self.capacity.max_runners {
            return Ok(false);
        }
        let Some(snapshot) = self.store.fleet_revision_latest(&self.fleet_key).await? else {
            return Ok(false);
        };
        if snapshot.revision != self.guard.desired_revision || snapshot.auth_desired != self.auth {
            return Ok(false);
        }
        let spec: shaula_core::fleet::FleetSpec = serde_json::from_str(&snapshot.spec_json)
            .map_err(|_| CoreError::new(ReasonCode::SpecInvalid, "stored Fleet spec invalid"))?;
        if spec.kind != FleetProviderKind::Forgejo {
            return Ok(false);
        }
        let (
            Some(profile_key),
            Some(template_revision),
            Some(artifact_digest),
            Some(attestation_id),
        ) = (
            snapshot.template_profile_key,
            snapshot.template_revision,
            snapshot.template_artifact_digest,
            snapshot.template_attestation_id,
        )
        else {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "Forgejo revision has no resolved template pin",
            ));
        };
        let generation_id = shaula_core::auth::new_attempt_id();
        let generation_name = resource_name(&generation_id);
        let runner_name = format!("{}{}", self.runner_name_prefix, generation_id);
        let workspace = self.work_root.join(&self.fleet_key).join(&generation_id);
        let record = GenerationRecord {
            id: generation_id.clone(),
            fleet_key: self.fleet_key.clone(),
            runner_name: runner_name.clone(),
            generation_name: generation_name.clone(),
            fleet_revision: snapshot.revision,
            template_profile_key: profile_key.clone(),
            template_revision,
            template_artifact_digest: artifact_digest.clone(),
            attestation_id,
            inputs_digest: snapshot.inputs_digest,
            state: GenerationState::CreatePending,
            github_runner_id: None,
            workspace_path: workspace.to_string_lossy().into_owned(),
            created_at: now,
            updated_at: now,
        };
        self.lifecycle.generation_insert(record.clone()).await?;
        self.lifecycle
            .generation_advance(&generation_id, GenerationState::Creating, now)
            .await?;
        drop(gate);
        let prepared = self.prepare_generation(&record).await;
        let (artifact_dir, manifest, bindings, bindings_digest) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return cleanup_error(self, &generation_id, now, error).await,
        };
        let Some(bootstrap) = self
            .register_generation(&record, self.clock.now_unix_ms())
            .await?
        else {
            return Ok(false);
        };
        let mut input = shaula_core::template::ShaulaInputEnvelope::new(
            shaula_core::template::GenerationIdentity {
                fleet_key: self.fleet_key.clone(),
                scale_set_id: None,
                id: generation_id.clone(),
                runner_name,
                generation_name,
            },
            String::new(),
            shaula_core::template::BindingsDigest(bindings_digest.clone()),
        );
        input.bindings = bindings;
        input.forgejo = Some(bootstrap.identity());
        input.parameters = spec.template_inputs;
        if manifest.input_contract_version == 2 {
            let descriptor = match self.setup_info_issuer.as_deref() {
                Some(issuer) => issuer
                    .issue(&generation_id, self.clock.now_unix_ms().div_euclid(1000))
                    .await
                    .unwrap_or(shaula_core::template::SetupInfoDescriptor::Disabled),
                None => shaula_core::template::SetupInfoDescriptor::Disabled,
            };
            input = match input.with_setup_info(descriptor) {
                Ok(input) => input,
                Err(error) => return cleanup_error(self, &generation_id, now, error).await,
            };
        }
        if let Err(error) = input.validate_for_manifest(&manifest) {
            return cleanup_error(self, &generation_id, now, error).await;
        }
        let apply_intent =
            crate::apply_intent::TrackedApplyIntentSink::new(self.apply_intent_sink.clone());
        let request = TemplateCreateRequest {
            workspace_path: workspace,
            artifact_dir,
            pinned_artifact_digest: artifact_digest,
            input,
            expected_bindings_digest: shaula_core::template::BindingsDigest(bindings_digest),
            managed_shape: manifest.managed_resource_shape,
            environment: Vec::new(),
            timeout: self.operation_timeout,
            apply_intent_sink: Some(apply_intent.clone()),
            forgejo_bootstrap: Some(bootstrap),
        };
        match self.runtime.create(request).await {
            Ok(result) => {
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
                    .generation_set_result(&generation_id, &result_json, &digest, now)
                    .await?;
                apply_intent.complete(self.lifecycle.as_ref(), now).await?;
                self.lifecycle
                    .generation_advance(&generation_id, GenerationState::WaitingOnline, now)
                    .await?;
                Ok(true)
            }
            Err(_) => {
                TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Failed, 1);
                self.lifecycle
                    .generation_advance(
                        &generation_id,
                        GenerationState::CleanupRequired,
                        self.clock.now_unix_ms(),
                    )
                    .await?;
                Ok(false)
            }
        }
    }

    async fn prepare_generation(
        &self,
        record: &GenerationRecord,
    ) -> CoreResult<(
        std::path::PathBuf,
        shaula_core::template::ProfileManifest,
        serde_json::Map<String, serde_json::Value>,
        String,
    )> {
        self.store
            .ensure_artifact_cached(&record.template_artifact_digest)
            .await?;
        let artifact_dir = shaula_core::artifact_layout::artifact_dir(
            &self.artifact_root,
            &record.template_artifact_digest,
        )
        .ok_or_else(|| {
            CoreError::new(
                ReasonCode::TemplateInvalid,
                "Forgejo artifact digest malformed",
            )
        })?;
        let manifest = read_manifest(&artifact_dir)?;
        manifest.validate_forgejo_targets(&self.bootstrap_labels)?;
        let Some((bindings_json, bindings_digest)) = self
            .store
            .template_protected_bindings(&record.template_profile_key, record.template_revision)
            .await?
        else {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "admitted bindings missing",
            ));
        };
        let bindings = serde_json::from_str(&bindings_json).map_err(|_| {
            CoreError::new(ReasonCode::TemplateInvalid, "admitted bindings invalid")
        })?;
        let workspace = std::path::Path::new(&record.workspace_path);
        tokio::fs::create_dir_all(workspace).await.map_err(|_| {
            CoreError::new(ReasonCode::Internal, "Forgejo workspace creation failed")
        })?;
        self.runtime
            .prepare_create(
                workspace,
                &artifact_dir,
                &record.template_artifact_digest,
                self.operation_timeout,
            )
            .await
            .map_err(|_| {
                CoreError::new(
                    ReasonCode::TemplateInvalid,
                    "Forgejo workspace preparation failed",
                )
            })?;
        Ok((artifact_dir, manifest, bindings, bindings_digest))
    }
}

fn resource_name(generation_id: &str) -> String {
    format!(
        "s{}",
        &hex::encode(Sha256::digest(generation_id.as_bytes()))[..56]
    )
}

pub(super) fn read_manifest(
    artifact_dir: &std::path::Path,
) -> CoreResult<shaula_core::template::ProfileManifest> {
    let body = std::fs::read_to_string(artifact_dir.join("profile.yaml"))
        .map_err(|_| CoreError::new(ReasonCode::TemplateInvalid, "admitted manifest unreadable"))?;
    let manifest: shaula_core::template::ProfileManifest = serde_yaml::from_str(&body)
        .map_err(|_| CoreError::new(ReasonCode::TemplateInvalid, "admitted manifest invalid"))?;
    manifest.validate()?;
    Ok(manifest)
}

async fn cleanup_error<T>(
    supervisor: &ForgejoPoolSupervisor,
    id: &str,
    now: i64,
    error: CoreError,
) -> CoreResult<T> {
    supervisor
        .lifecycle
        .generation_advance(id, GenerationState::CleanupRequired, now)
        .await?;
    Err(error)
}
