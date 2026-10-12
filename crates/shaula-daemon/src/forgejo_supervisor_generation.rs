//! Durable Forgejo Generation creation. Prepare retained inputs before registration.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetProviderKind;
use shaula_core::lifecycle::GenerationState;
use shaula_core::registry::GenerationRecord;

use super::ForgejoPoolSupervisor;
use stages::{GenerationNames, PinnedRevision, PreparedGeneration};

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
        // Reserve occupancy under the same exclusive gate used by Fleet writes.
        let gate = self.gates.acquire_exclusive(&self.fleet_key).await;
        let Some((snapshot, spec)) = self.admissible_revision().await? else {
            return Ok(false);
        };
        let pooled = spec.template_pool.is_some() || spec.template_pool_ref.is_some();
        let pinned = PinnedRevision::from_snapshot(pooled, snapshot)?;
        let names = GenerationNames::mint(self);
        let record = names.record(&self.fleet_key, pinned, now);
        let Some((record, parameters)) = self
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
        let prepared = match self.prepare_generation(&record, &parameters).await {
            Ok(prepared) => prepared,
            Err(error) => return cleanup_error(self, &generation_id, now, error).await,
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
            Err(error) => return cleanup_error(self, &generation_id, now, error).await,
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
        self.record_create_outcome(
            &generation_id,
            self.runtime.create(request).await,
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

    async fn prepare_generation(
        &self,
        record: &GenerationRecord,
        parameters: &serde_json::Map<String, serde_json::Value>,
    ) -> CoreResult<PreparedGeneration> {
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
        manifest.validate_bindings_for_provider(&bindings, FleetProviderKind::Forgejo)?;
        if manifest.container_bootstrap_contract.is_some() {
            manifest.selected_runner_image(&bindings, parameters)?;
        }
        let workspace = std::path::Path::new(&record.workspace_path);
        tokio::fs::create_dir_all(workspace)
            .await
            .map_err(|_| CoreError::internal("Forgejo workspace creation failed"))?;
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
        Ok(PreparedGeneration {
            artifact_dir,
            manifest,
            bindings,
            bindings_digest,
        })
    }
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
