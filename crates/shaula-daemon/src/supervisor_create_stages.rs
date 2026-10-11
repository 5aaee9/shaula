//! Named stages of [`FleetSupervisor::create_one_generation`]: admission
//! reads, durable generation identity, the pre-JIT fence, local workspace
//! preparation, the JIT mint and the frozen create request.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetSpec;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::{JitConfig, TemplateCreateRequest};
use shaula_core::registry::{FleetHead, FleetRevisionRow, FleetRuntimeGuard, GenerationRecord};
use shaula_core::template::{
    BindingsDigest, GenerationIdentity, ProfileManifest, ShaulaInputEnvelope,
};

use super::{fingerprint, setup_info, FleetSupervisor};
use crate::apply_intent::TrackedApplyIntentSink;

/// Frozen template pin carried by a non-pool generation record.
pub(super) struct TemplatePin {
    profile: String,
    revision: i64,
    artifact: String,
    attestation: String,
}

/// Whether the fleet selects its template through a pool (legacy inline
/// pool or a shared `template_pool_ref`, spec 0037).
pub(super) fn uses_template_pool(spec: &FleetSpec) -> bool {
    spec.template_pool.is_some() || spec.template_pool_ref.is_some()
}

/// The template pin frozen into the generation record. Pool admission
/// selects and freezes the member atomically later, so pooled fleets start
/// with an empty pin.
pub(super) fn resolve_pin(pooled: bool, snapshot: &FleetRevisionRow) -> CoreResult<TemplatePin> {
    if pooled {
        return Ok(TemplatePin {
            profile: String::new(),
            revision: 0,
            artifact: String::new(),
            attestation: String::new(),
        });
    }
    match (
        &snapshot.template_profile_key,
        snapshot.template_revision,
        &snapshot.template_artifact_digest,
        &snapshot.template_attestation_id,
    ) {
        (Some(k), Some(r), Some(d), Some(a)) => Ok(TemplatePin {
            profile: k.clone(),
            revision: r,
            artifact: d.clone(),
            attestation: a.clone(),
        }),
        _ => Err(CoreError::internal(
            "admitted fleet revision has no resolved template pin",
        )),
    }
}

/// Durable identity of one new generation.
pub(super) struct GenerationNames {
    pub(super) generation_id: String,
    runner_name: String,
    generation_name: String,
    pub(super) workspace: PathBuf,
}

impl GenerationNames {
    pub(super) fn mint(fleet_key: &str, work_root: &Path) -> CoreResult<Self> {
        let generation_id = shaula_core::auth::new_attempt_id();
        // The short display runner_name must never become a platform
        // resource name.
        let generation_name = crate::runner_operation::generation_resource_name(&generation_id);
        if generation_name.len() > 63 {
            // Unreachable by construction; fail closed rather than emit a
            // name Kubernetes would reject or that could collide.
            return Err(CoreError::internal(
                "generation name exceeds the Kubernetes 63-char limit",
            ));
        }
        let runner_name = format!(
            "shaula-{}-{}",
            fleet_key,
            &generation_id[..8.min(generation_id.len())]
        );
        let workspace = work_root.join(fleet_key).join(&generation_id);
        Ok(Self {
            generation_id,
            runner_name,
            generation_name,
            workspace,
        })
    }

    /// The CreatePending ledger record derived from the ONE admitted
    /// revision snapshot.
    pub(super) fn record(
        &self,
        fleet_key: &str,
        snapshot: &FleetRevisionRow,
        pin: TemplatePin,
        now: i64,
    ) -> GenerationRecord {
        GenerationRecord {
            id: self.generation_id.clone(),
            fleet_key: fleet_key.to_owned(),
            runner_name: self.runner_name.clone(),
            generation_name: self.generation_name.clone(),
            fleet_revision: snapshot.revision,
            template_profile_key: pin.profile,
            pool_member_key: None,
            template_revision: pin.revision,
            template_artifact_digest: pin.artifact,
            attestation_id: pin.attestation,
            inputs_digest: snapshot.inputs_digest.clone(),
            state: GenerationState::CreatePending,
            github_runner_id: None,
            workspace_path: self.workspace.to_string_lossy().into_owned(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Everything the frozen create request is assembled from.
pub(super) struct CreateInputs {
    pub(super) names: GenerationNames,
    pub(super) scale_set_id: i64,
    pub(super) jit: JitConfig,
    pub(super) material: (ProfileManifest, Map<String, Value>, String),
    pub(super) parameters: Map<String, Value>,
    pub(super) artifact_dir: PathBuf,
}

impl FleetSupervisor {
    /// Route-proof gate (spec 0011 §5.3): a new Create/JIT effect runs
    /// only on fresh authorization evidence; unprovable routes fail
    /// closed without touching safe cleanup.
    pub(super) async fn route_proof_admits(&self) -> bool {
        if let Err(failure) = self.github.ensure_route_proof().await {
            tracing::warn!(
                fleet = %self.config.fleet_key,
                summary = %failure.summary(),
                "route proof unavailable; create effects blocked"
            );
            return false;
        }
        true
    }

    /// Create-claim admission gate plus THE admitted-revision snapshot.
    /// `None` retries on the next tick.
    pub(super) async fn admitted_snapshot(
        &self,
    ) -> CoreResult<Option<(FleetHead, FleetRevisionRow)>> {
        // A decommissioning or tombstoned fleet must never spawn new Creates.
        let head = self.handoff.fleet_get(&self.config.fleet_key).await?;
        let head = match head {
            Some(h) if !h.deletion_marker && !h.tombstone => h,
            _ => return Ok(None),
        };
        // The whole generation (ledger record, envelope parameters, pin,
        // inputs digest) is derived from this ONE immutable row — never
        // from constructor-time copies or a later `latest` re-read that
        // could observe a newer revision (spec 0001 §10.2.1: one Fleet
        // Revision per generation).
        let snapshot = self
            .handoff
            .fleet_revision_latest(&self.config.fleet_key)
            .await?
            .ok_or_else(|| CoreError::internal("fleet has no admitted revision"))?;
        if snapshot.revision != head.desired_revision {
            // Head moved between the two reads: nothing durable happened
            // yet; retry on the next tick against the settled head.
            return Ok(None);
        }
        Ok(Some((head, snapshot)))
    }

    /// Durably inserts the generation row (pool admission or guarded
    /// insert). Returns the stored record and its envelope parameters;
    /// `None` retries on the next tick without creating a row.
    pub(super) async fn admit_generation(
        &self,
        record: GenerationRecord,
        head: &FleetHead,
        pooled: bool,
        parameters: Map<String, Value>,
    ) -> CoreResult<Option<(GenerationRecord, Map<String, Value>)>> {
        let guard = FleetRuntimeGuard::from(head);
        if pooled {
            let Some(admission) = self.store.generation_admit_pool(record, &guard).await? else {
                // No eligible member or a concurrent revision/fence change.
                return Ok(None);
            };
            return Ok(Some((admission.generation, admission.template_inputs)));
        }
        if !self
            .store
            .generation_insert_guarded(record.clone(), &guard)
            .await?
        {
            // The fleet changed after the snapshot but before the
            // durable generation row could be created. Retry against
            // the new head without leaving a stale generation behind.
            return Ok(None);
        }
        Ok(Some((record, parameters)))
    }

    /// Durable ownership FIRST: the exact bound Scale Set ID must be
    /// read and validated before JITStarting (spec 0001 §10.2). An
    /// unbound fleet cannot mint JIT for a generation.
    pub(super) async fn bound_scale_set_id(&self) -> CoreResult<i64> {
        let ownership = self.store.scale_set_get(&self.config.fleet_key).await?;
        ownership
            .as_ref()
            .and_then(|s| s.scale_set_id)
            .ok_or_else(|| {
                CoreError::new(ReasonCode::OwnershipProofFailed, "scale set is not bound")
            })
    }

    /// Fence re-check immediately before the FIRST remote effect: if a
    /// newer fleet revision (or a DELETE) won the race since the
    /// snapshot was taken, this generation must never mint JIT. Only
    /// local rows exist so far — retreat without quarantine.
    pub(super) async fn fence_holds(
        &self,
        generation_id: &str,
        revision: i64,
        now: i64,
    ) -> CoreResult<bool> {
        let head_now = self.handoff.fleet_get(&self.config.fleet_key).await?;
        if head_now
            .as_ref()
            .map(|h| h.desired_revision != revision || h.deletion_marker)
            .unwrap_or(true)
        {
            self.store
                .generation_advance(generation_id, GenerationState::Quarantined, now)
                .await?;
            return Ok(false);
        }
        Ok(true)
    }

    /// R9-07 (spec 0004 §6): LOCAL, retryable work comes FIRST — the
    /// workspace materializes and the LOCKED init runs before any
    /// remote effect, so a materialization failure can never burn a
    /// JIT token. The runtime's later `create` re-verifies the
    /// prepared material. Returns the cached artifact directory.
    pub(super) async fn prepare_workspace(
        &self,
        record: &GenerationRecord,
        workspace: &Path,
    ) -> CoreResult<PathBuf> {
        self.handoff
            .ensure_artifact_cached(&record.template_artifact_digest)
            .await?;
        let artifact_dir = shaula_core::artifact_layout::artifact_dir(
            &self.config.artifact_root,
            &record.template_artifact_digest,
        )
        .ok_or_else(|| {
            CoreError::internal(format!(
                "artifact digest malformed: {}",
                record.template_artifact_digest
            ))
        })?;
        std::fs::create_dir_all(workspace)
            .map_err(|e| CoreError::internal(format!("workspace unusable: {e}")))?;
        self.runtime
            .prepare_create(
                workspace,
                &artifact_dir,
                &record.template_artifact_digest,
                self.config.operation_timeout,
            )
            .await
            .map_err(|_| CoreError::internal("workspace prepare failed"))?;
        Ok(artifact_dir)
    }

    /// Durable JIT intent BEFORE the remote JIT request (R9-07/R10-09,
    /// spec 0004 §6): phase marker plus a JitStarting operation row
    /// freezing the COMPLETE admission context — the Auth Revision Ref
    /// this supervisor was built with, a unique JIT attempt id, a
    /// request digest, and the full pin tuple — so a lost response
    /// leaves recovery evidence that proves WHICH admission-time
    /// authority minted the JIT, instead of an invisible half-effect.
    pub(super) async fn mint_generation_jit(
        &self,
        names: &GenerationNames,
        record: &GenerationRecord,
        scale_set_id: i64,
        now: i64,
    ) -> CoreResult<Option<JitConfig>> {
        let jit_attempt_id = shaula_core::auth::new_attempt_id();
        let jit_context = serde_json::json!({
            "fleet_key": self.config.fleet_key,
            "generation_id": names.generation_id,
            "runner_name": names.runner_name,
            "generation_name": names.generation_name,
            "scale_set_id": scale_set_id,
            "template_profile": record.template_profile_key,
            "template_revision": record.template_revision,
            "artifact_digest": record.template_artifact_digest,
            "attestation_id": record.attestation_id,
            "auth_profile_key": self.config.auth_profile_key,
            "auth_revision": self.config.auth_revision,
            "identity_fingerprint": fingerprint(&self.identity),
            "jit_attempt_id": jit_attempt_id,
        });
        self.acquire_generation_jit(
            &names.generation_id,
            scale_set_id,
            &names.runner_name,
            jit_context,
            jit_attempt_id,
            now,
        )
        .await
    }

    /// The frozen protected envelope and the create request carrying it:
    /// bindings come from the ADMITTED Template Revision (protected-memory
    /// handoff), parameters from the SAME admitted Fleet Revision snapshot
    /// that populated the ledger record — never a `latest` re-read after a
    /// remote effect (spec 0001 §10.2.3, 0004 §6.1).
    pub(super) async fn create_request(
        &self,
        inputs: CreateInputs,
        record: &GenerationRecord,
    ) -> CoreResult<(TemplateCreateRequest, Arc<TrackedApplyIntentSink>)> {
        let CreateInputs {
            names,
            scale_set_id,
            jit,
            material: (manifest, bindings, bindings_digest),
            parameters,
            artifact_dir,
        } = inputs;
        let managed_shape = manifest.managed_resource_shape.clone();
        let mut input = ShaulaInputEnvelope::new(
            GenerationIdentity {
                fleet_key: self.config.fleet_key.clone(),
                scale_set_id: Some(scale_set_id),
                id: names.generation_id,
                runner_name: names.runner_name,
                generation_name: names.generation_name,
            },
            jit.encoded,
            BindingsDigest(bindings_digest.clone()),
        );
        input.bindings = bindings;
        input.parameters = parameters;
        if manifest.input_contract_version == 2 {
            let descriptor = setup_info::issue(
                self.setup_info_issuer.as_deref(),
                self.clock.as_deref(),
                &input.generation.id,
            )
            .await;
            input = input.with_setup_info(descriptor)?;
        }
        input.validate_for_manifest(&manifest)?;
        let apply_intent = TrackedApplyIntentSink::new(self.config.apply_intent_sink.clone());
        let request = TemplateCreateRequest {
            workspace_path: names.workspace,
            artifact_dir,
            pinned_artifact_digest: record.template_artifact_digest.clone(),
            input,
            expected_bindings_digest: BindingsDigest(bindings_digest),
            managed_shape,
            environment: Vec::new(),
            timeout: self.config.operation_timeout,
            apply_intent_sink: Some(apply_intent.clone()),
            forgejo_bootstrap: None,
        };
        Ok((request, apply_intent))
    }
}
