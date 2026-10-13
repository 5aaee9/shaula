//! Pool Fleet Generation create stages shared by the Forgejo and GitLab
//! drivers: the gated admission read, the frozen pin and identity, the
//! durable row, workspace preparation and the recorded apply outcome. Only
//! registration and the protected bootstrap request are backend-specific.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use shaula_core::capacity::CapacityPolicy;
use shaula_core::diagnostics::{Capture, Code, Outcome, StageId};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::{FleetProviderKind, FleetSpec};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::{Clock, TemplateCreateResult, TemplateOutcomeError, TemplateRuntimePort};
use shaula_core::registry::{
    ControlPlaneStore, FleetRevisionRow, FleetRuntimeGuard, GenerationRecord, LifecycleStore,
};
use shaula_core::template::ProfileManifest;
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use crate::apply_intent::TrackedApplyIntentSink;

/// The exact Fleet authority and dependencies one Pool create runs under.
pub(crate) struct PoolCreate<'a> {
    pub fleet_key: &'a str,
    pub guard: &'a FleetRuntimeGuard,
    pub auth: &'a (String, i64),
    pub capacity: CapacityPolicy,
    pub provider: FleetProviderKind,
    pub store: &'a dyn ControlPlaneStore,
    pub lifecycle: &'a dyn LifecycleStore,
    pub runtime: &'a dyn TemplateRuntimePort,
    pub clock: &'a dyn Clock,
    pub artifact_root: &'a Path,
    pub operation_timeout: Duration,
}

/// The admitted revision fields frozen into the generation record.
pub(crate) struct PinnedRevision {
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
    pub(crate) fn from_snapshot(pooled: bool, snapshot: FleetRevisionRow) -> CoreResult<Self> {
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
                        "Pool revision has no resolved template pin",
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

/// Durable identity of one new Pool generation.
pub(crate) struct GenerationNames {
    pub(crate) generation_id: String,
    pub(crate) runner_name: String,
    pub(crate) generation_name: String,
    pub(crate) workspace: PathBuf,
}

impl GenerationNames {
    pub(crate) fn mint(fleet_key: &str, runner_name_prefix: &str, work_root: &Path) -> Self {
        let generation_id = shaula_core::auth::new_attempt_id();
        let generation_name = crate::runner_operation::generation_resource_name(&generation_id);
        let runner_name = format!("{runner_name_prefix}{generation_id}");
        let workspace = work_root.join(fleet_key).join(&generation_id);
        Self {
            generation_id,
            runner_name,
            generation_name,
            workspace,
        }
    }

    pub(crate) fn record(
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

/// Retained local material from [`PoolCreate::prepare_generation`].
pub(crate) struct PreparedGeneration {
    pub(crate) artifact_dir: PathBuf,
    pub(crate) manifest: ProfileManifest,
    pub(crate) bindings: Map<String, Value>,
    pub(crate) bindings_digest: String,
}

impl PoolCreate<'_> {
    /// Admission reads under the exclusive Fleet gate. Never borrow a newer
    /// revision with a client built for an older one; `None` skips this
    /// create without any durable effect.
    pub(crate) async fn admissible_revision(
        &self,
    ) -> CoreResult<Option<(FleetRevisionRow, FleetSpec)>> {
        let current = self
            .store
            .fleet_get(self.fleet_key)
            .await?
            .filter(|head| !head.tombstone && FleetRuntimeGuard::from(head) == *self.guard);
        if current.is_none_or(|head| head.deletion_marker) {
            return Ok(None);
        }
        if self.store.generations_occupancy(self.fleet_key).await? >= self.capacity.max_runners {
            return Ok(None);
        }
        let Some(snapshot) = self.store.fleet_revision_latest(self.fleet_key).await? else {
            return Ok(None);
        };
        if snapshot.revision != self.guard.desired_revision || snapshot.auth_desired != *self.auth {
            return Ok(None);
        }
        let spec: FleetSpec = serde_json::from_str(&snapshot.spec_json)
            .map_err(|_| CoreError::new(ReasonCode::SpecInvalid, "stored Fleet spec invalid"))?;
        if spec.kind != self.provider {
            return Ok(None);
        }
        Ok(Some((snapshot, spec)))
    }

    /// Inserts the durable generation row (pool admission or guarded
    /// insert); returns the stored record and its envelope parameters.
    pub(crate) async fn admit_generation(
        &self,
        record: GenerationRecord,
        pooled: bool,
        parameters: Map<String, Value>,
    ) -> CoreResult<Option<(GenerationRecord, Map<String, Value>)>> {
        if pooled {
            let Some(admission) = self
                .lifecycle
                .generation_admit_pool(record, self.guard)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((admission.generation, admission.template_inputs)));
        }
        if !self
            .lifecycle
            .generation_insert_guarded(record.clone(), self.guard)
            .await?
        {
            return Ok(None);
        }
        Ok(Some((record, parameters)))
    }

    /// Verifies the admitted artifact and bindings and prepares the
    /// workspace. `check` adds the backend's manifest target validation.
    pub(crate) async fn prepare_generation(
        &self,
        record: &GenerationRecord,
        parameters: &Map<String, Value>,
        check: impl FnOnce(&ProfileManifest) -> CoreResult<()>,
    ) -> CoreResult<PreparedGeneration> {
        self.store
            .ensure_artifact_cached(&record.template_artifact_digest)
            .await?;
        let artifact_dir = shaula_core::artifact_layout::artifact_dir(
            self.artifact_root,
            &record.template_artifact_digest,
        )
        .ok_or_else(|| {
            CoreError::new(
                ReasonCode::TemplateInvalid,
                "Pool artifact digest malformed",
            )
        })?;
        let manifest = read_manifest(&artifact_dir)?;
        check(&manifest)?;
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
        manifest.validate_bindings_for_provider(&bindings, self.provider)?;
        if manifest.container_bootstrap_contract.is_some() {
            manifest.selected_runner_image(&bindings, parameters)?;
        }
        let workspace = Path::new(&record.workspace_path);
        tokio::fs::create_dir_all(workspace)
            .await
            .map_err(|_| CoreError::internal("Pool workspace creation failed"))?;
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
                    "Pool workspace preparation failed",
                )
            })?;
        Ok(PreparedGeneration {
            artifact_dir,
            manifest,
            bindings,
            bindings_digest,
        })
    }

    /// Records the apply outcome: result + WaitingOnline on success,
    /// CleanupRequired on failure.
    pub(crate) async fn record_create_outcome(
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
                apply_intent.complete(self.lifecycle, now).await?;
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

    /// A pre-runtime failure keeps its cleanup evidence and surfaces `error`.
    pub(crate) async fn cleanup_error<T>(
        &self,
        id: &str,
        now: i64,
        error: CoreError,
    ) -> CoreResult<T> {
        self.lifecycle
            .generation_advance(id, GenerationState::CleanupRequired, now)
            .await?;
        Err(error)
    }
}

pub(crate) fn read_manifest(artifact_dir: &Path) -> CoreResult<ProfileManifest> {
    let body = std::fs::read_to_string(artifact_dir.join("profile.yaml"))
        .map_err(|_| CoreError::new(ReasonCode::TemplateInvalid, "admitted manifest unreadable"))?;
    let manifest: ProfileManifest = serde_yaml::from_str(&body)
        .map_err(|_| CoreError::new(ReasonCode::TemplateInvalid, "admitted manifest invalid"))?;
    manifest.validate()?;
    Ok(manifest)
}
