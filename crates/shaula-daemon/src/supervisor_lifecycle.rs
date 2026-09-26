//! Runner generation lifecycle: create/retire/destroy flows as inherent
//! methods on [`FleetSupervisor`]. The submodule keeps the host file within
//! the 400-line limit while methods keep private-field access.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::ports::TemplateCreateRequest;
use shaula_core::registry::{FleetRuntimeGuard, GenerationRecord};
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use super::{fingerprint, FleetSupervisor};

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
        use shaula_core::diagnostics::{Code, Guard, Lane, Observer, QuestionId};
        let observer = self.runtime_guard.as_ref().and_then(|guard| {
            Observer::register(
                self.handoff.diagnostic_sink(),
                Guard::fleet(&self.config.fleet_key, guard),
                Lane::Operation,
                QuestionId::ScaleUp,
            )
        });
        let _permit = crate::diagnostic_capture::acquire(
            &self.limits.create,
            observer,
            Code::ExecutionCreateSlotWait,
            now,
        )
        .await
        .map_err(|_| CoreError::new(ReasonCode::Internal, "create scheduler stopped"))?;
        // Route-proof gate (spec 0011 §5.3): a new Create/JIT effect runs
        // only on fresh authorization evidence; unprovable routes fail
        // closed without touching safe cleanup.
        if let Err(failure) = self.github.ensure_route_proof().await {
            tracing::warn!(
                fleet = %self.config.fleet_key,
                summary = %failure.summary(),
                "route proof unavailable; create effects blocked"
            );
            return Ok(false);
        }
        // Create-claim admission gate: a decommissioning or tombstoned
        // fleet must never spawn new Creates.
        let head = self.handoff.fleet_get(&self.config.fleet_key).await?;
        let head = match head {
            Some(h) if !h.deletion_marker && !h.tombstone => h,
            _ => return Ok(false),
        };
        // THE admitted-revision snapshot: the whole generation (ledger
        // record, envelope parameters, pin, inputs digest) is derived from
        // this ONE immutable row — never from constructor-time copies or a
        // later `latest` re-read that could observe a newer revision
        // (spec 0001 §10.2.1: one Fleet Revision per generation).
        let snapshot = self
            .handoff
            .fleet_revision_latest(&self.config.fleet_key)
            .await?
            .ok_or_else(|| {
                CoreError::new(ReasonCode::Internal, "fleet has no admitted revision")
            })?;
        if snapshot.revision != head.desired_revision {
            // Head moved between the two reads: nothing durable happened
            // yet; retry on the next tick against the settled head.
            return Ok(false);
        }
        let spec: shaula_core::fleet::FleetSpec = serde_json::from_str(&snapshot.spec_json)
            .map_err(|e| CoreError::new(ReasonCode::Internal, format!("spec invalid: {e}")))?;
        let mut parameters = spec.template_inputs;
        let (pin_profile, pin_revision, pin_artifact, pin_attestation) =
            if spec.template_pool.is_some() || spec.template_pool_ref.is_some() {
                // Pool admission selects and freezes the member atomically
                // below — for both the legacy inline pool and a shared
                // `template_pool_ref` (spec 0037).
                (String::new(), 0, String::new(), String::new())
            } else {
                match (
                    &snapshot.template_profile_key,
                    snapshot.template_revision,
                    &snapshot.template_artifact_digest,
                    &snapshot.template_attestation_id,
                ) {
                    (Some(k), Some(r), Some(d), Some(a)) => (k.clone(), r, d.clone(), a.clone()),
                    _ => {
                        return Err(CoreError::new(
                            ReasonCode::Internal,
                            "admitted fleet revision has no resolved template pin",
                        ))
                    }
                }
            };

        let generation_id = shaula_core::auth::new_attempt_id();
        // Kubernetes names cap at 63 chars: derive the generation-scoped
        // resource name from sha256(generation_id), keeping 56 hex chars
        // (224 bits — collision resistance stays cryptographic). The short
        // display runner_name must never become a platform resource name.
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest as _;
        hasher.update(generation_id.as_bytes());
        let generation_name = format!("s{}", &hex::encode(hasher.finalize())[..56]);
        if generation_name.len() > 63 {
            // Unreachable by construction; fail closed rather than emit a
            // name Kubernetes would reject or that could collide.
            return Err(CoreError::new(
                ReasonCode::Internal,
                "generation name exceeds the Kubernetes 63-char limit",
            ));
        }
        let runner_name = format!(
            "shaula-{}-{}",
            self.config.fleet_key,
            &generation_id[..8.min(generation_id.len())]
        );
        let workspace = self
            .config
            .work_root
            .join(&self.config.fleet_key)
            .join(&generation_id);

        let record = GenerationRecord {
            id: generation_id.clone(),
            fleet_key: self.config.fleet_key.clone(),
            runner_name: runner_name.clone(),
            generation_name: generation_name.clone(),
            fleet_revision: snapshot.revision,
            template_profile_key: pin_profile.clone(),
            pool_member_key: None,
            template_revision: pin_revision,
            template_artifact_digest: pin_artifact.clone(),
            attestation_id: pin_attestation.clone(),
            inputs_digest: snapshot.inputs_digest.clone(),
            state: shaula_core::lifecycle::GenerationState::CreatePending,
            github_runner_id: None,
            workspace_path: workspace.to_string_lossy().into_owned(),
            created_at: now,
            updated_at: now,
        };
        let record = if spec.template_pool.is_some() || spec.template_pool_ref.is_some() {
            let guard = FleetRuntimeGuard::from(&head);
            let Some(admission) = self.store.generation_admit_pool(record, &guard).await? else {
                // No eligible member or a concurrent revision/fence change.
                // The next convergence tick retries without creating a row.
                return Ok(false);
            };
            parameters = admission.template_inputs;
            admission.generation
        } else {
            let guard = FleetRuntimeGuard::from(&head);
            if !self
                .store
                .generation_insert_guarded(record.clone(), &guard)
                .await?
            {
                // The fleet changed after the snapshot but before the
                // durable generation row could be created. Retry against
                // the new head without leaving a stale generation behind.
                return Ok(false);
            }
            record
        };
        if record.state == shaula_core::lifecycle::GenerationState::CreatePending {
            self.store
                .generation_advance(
                    &generation_id,
                    shaula_core::lifecycle::GenerationState::Creating,
                    now,
                )
                .await?;
        }

        // Durable ownership FIRST: the exact bound Scale Set ID must be
        // read and validated before JITStarting (spec 0001 §10.2). An
        // unbound fleet cannot mint JIT for a generation.
        let ownership = self.store.scale_set_get(&self.config.fleet_key).await?;
        let scale_set_id = ownership
            .as_ref()
            .and_then(|s| s.scale_set_id)
            .ok_or_else(|| {
                CoreError::new(ReasonCode::OwnershipProofFailed, "scale set is not bound")
            })?;

        // Fence re-check immediately before the FIRST remote effect: if a
        // newer fleet revision (or a DELETE) won the race since the
        // snapshot was taken, this generation must never mint JIT. Only
        // local rows exist so far — retreat without quarantine.
        let head_now = self.handoff.fleet_get(&self.config.fleet_key).await?;
        if head_now
            .as_ref()
            .map(|h| h.desired_revision != snapshot.revision || h.deletion_marker)
            .unwrap_or(true)
        {
            self.store
                .generation_advance(
                    &generation_id,
                    shaula_core::lifecycle::GenerationState::Quarantined,
                    now,
                )
                .await?;
            return Ok(false);
        }

        // R9-07 (spec 0004 §6): LOCAL, retryable work comes FIRST — the
        // workspace materializes and the LOCKED init runs before any
        // remote effect, so a materialization failure can never burn a
        // JIT token. The runtime's later `create` re-verifies the
        // prepared material.
        self.handoff
            .ensure_artifact_cached(&record.template_artifact_digest)
            .await?;
        let artifact_dir = shaula_core::artifact_layout::artifact_dir(
            &self.config.artifact_root,
            &record.template_artifact_digest,
        )
        .ok_or_else(|| {
            CoreError::new(
                ReasonCode::Internal,
                format!(
                    "artifact digest malformed: {}",
                    record.template_artifact_digest
                ),
            )
        })?;
        std::fs::create_dir_all(&workspace).map_err(|e| {
            CoreError::new(ReasonCode::Internal, format!("workspace unusable: {e}"))
        })?;
        self.runtime
            .prepare_create(
                &workspace,
                &artifact_dir,
                &record.template_artifact_digest,
                self.config.operation_timeout,
            )
            .await
            .map_err(|_| CoreError::new(ReasonCode::Internal, "workspace prepare failed"))?;

        // The backend selection and images belong to the exact protected
        // Template Revision. Check them before minting any remote JIT.
        let (manifest, bindings, bindings_digest) = self
            .template_material(&artifact_dir, &record, &parameters)
            .await?;
        let managed_shape = manifest.managed_resource_shape.clone();

        // Durable JIT intent BEFORE the remote JIT request (R9-07/R10-09,
        // spec 0004 §6): phase marker plus a JitStarting operation row
        // freezing the COMPLETE admission context — the Auth Revision Ref
        // this supervisor was built with, a unique JIT attempt id, a
        // request digest, and the full pin tuple — so a lost response
        // leaves recovery evidence that proves WHICH admission-time
        // authority minted the JIT, instead of an invisible half-effect.
        let jit_attempt_id = shaula_core::auth::new_attempt_id();
        let jit_context = serde_json::json!({
            "fleet_key": self.config.fleet_key,
            "generation_id": generation_id,
            "runner_name": runner_name,
            "generation_name": generation_name,
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
        let Some(jit) = self
            .acquire_generation_jit(
                &generation_id,
                scale_set_id,
                &runner_name,
                jit_context,
                jit_attempt_id,
                now,
            )
            .await?
        else {
            return Ok(false);
        };
        // The frozen protected envelope: bindings come from the ADMITTED
        // Template Revision (protected-memory handoff), parameters from the
        // SAME admitted Fleet Revision snapshot that populated the ledger
        // record — never a `latest` re-read after a remote effect
        // (spec 0001 §10.2.3, 0004 §6.1).
        let mut input = shaula_core::template::ShaulaInputEnvelope::new(
            shaula_core::template::GenerationIdentity {
                fleet_key: self.config.fleet_key.clone(),
                scale_set_id: Some(scale_set_id),
                id: generation_id.clone(),
                runner_name,
                generation_name,
            },
            jit.encoded,
            shaula_core::template::BindingsDigest(bindings_digest.clone()),
        );
        input.bindings = bindings;
        input.parameters = parameters;
        if manifest.input_contract_version == 2 {
            let descriptor = setup_info::issue(
                self.setup_info_issuer.as_deref(),
                self.clock.as_deref(),
                &generation_id,
            )
            .await;
            input = input.with_setup_info(descriptor)?;
        }
        input.validate_for_manifest(&manifest)?;
        let apply_intent =
            crate::apply_intent::TrackedApplyIntentSink::new(self.config.apply_intent_sink.clone());
        let request = TemplateCreateRequest {
            workspace_path: workspace,
            artifact_dir,
            pinned_artifact_digest: record.template_artifact_digest.clone(),
            input,
            expected_bindings_digest: shaula_core::template::BindingsDigest(
                bindings_digest.clone(),
            ),
            managed_shape,
            environment: Vec::new(),
            timeout: self.config.operation_timeout,
            apply_intent_sink: Some(apply_intent.clone()),
            forgejo_bootstrap: None,
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
}
#[path = "supervisor_create_result.rs"]
mod create_result;
