//! Runner generation lifecycle: create/retire/destroy flows as inherent
//! methods on [`FleetSupervisor`]. The submodule keeps the host file within
//! the 400-line limit while methods keep private-field access.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::ports::TemplateCreateRequest;
use shaula_core::registry::{GenerationRecord, ScaleSetRow};

use super::{fingerprint, FleetSupervisor};

#[path = "supervisor_operations.rs"]
mod operations;

impl FleetSupervisor {
    pub(crate) async fn upsert_ownership(
        &self,
        scale_set_id: Option<i64>,
        state: &str,
        attempt_id: Option<String>,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .scale_set_upsert(ScaleSetRow {
                fleet_key: self.config.fleet_key.clone(),
                scale_set_id,
                name: self.identity.scale_set_name.clone(),
                runner_group: self.identity.runner_group.clone(),
                fingerprint: fingerprint(&self.identity),
                state: state.to_string(),
                attempt_id,
                now,
            })
            .await
    }

    /// Creates one runner generation: durable identity, then JIT (once), then
    /// apply (once) then WaitingOnline. The Create claim re-validates the
    /// fleet deletion marker before any external effect (0002 section 8).
    pub(crate) async fn create_one_generation(&self, now: i64) -> CoreResult<bool> {
        let _permit = self
            .limits
            .create
            .acquire()
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
        let parameters = spec.template_inputs;
        let (pin_profile, pin_revision, pin_artifact, pin_attestation) = match (
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
        self.store.generation_insert(record.clone()).await?;
        self.store
            .generation_advance(
                &generation_id,
                shaula_core::lifecycle::GenerationState::Creating,
                now,
            )
            .await?;

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
                &pin_artifact,
                self.config.operation_timeout,
            )
            .await
            .map_err(|_| CoreError::new(ReasonCode::Internal, "workspace prepare failed"))?;

        // Managed resource shape from the admitted artifact manifest,
        // read from the content-addressed artifact root.
        let manifest_path = artifact_dir.join("profile.yaml");
        let manifest: shaula_core::template::ProfileManifest =
            std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|m| {
                    serde_yaml::from_str::<shaula_core::template::ProfileManifest>(&m).ok()
                })
                .filter(|m: &shaula_core::template::ProfileManifest| m.validate().is_ok())
                .ok_or_else(|| {
                    CoreError::new(ReasonCode::TemplateInvalid, "admitted manifest unreadable")
                })?;
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
            "template_profile": pin_profile,
            "template_revision": pin_revision,
            "artifact_digest": pin_artifact,
            "attestation_id": pin_attestation,
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
        let (bindings_json, bindings_digest) = self
            .handoff
            .template_protected_bindings(&record.template_profile_key, record.template_revision)
            .await?
            .ok_or_else(|| {
                CoreError::new(ReasonCode::TemplateInvalid, "admitted bindings missing")
            })?;
        let bindings: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&bindings_json).map_err(|e| {
                CoreError::new(ReasonCode::Internal, format!("bindings invalid: {e}"))
            })?;
        let mut input = shaula_core::template::ShaulaInputEnvelope::new(
            shaula_core::template::GenerationIdentity {
                fleet_key: self.config.fleet_key.clone(),
                scale_set_id,
                id: generation_id.clone(),
                runner_name,
                generation_name,
            },
            jit.encoded,
            shaula_core::template::BindingsDigest(bindings_digest.clone()),
        );
        input.bindings = bindings;
        input.parameters = parameters;
        let apply_intent =
            crate::apply_intent::TrackedApplyIntentSink::new(self.config.apply_intent_sink.clone());
        let request = TemplateCreateRequest {
            workspace_path: workspace,
            artifact_dir,
            pinned_artifact_digest: pin_artifact,
            input,
            expected_bindings_digest: shaula_core::template::BindingsDigest(
                bindings_digest.clone(),
            ),
            managed_shape,
            environment: Vec::new(),
            timeout: self.config.operation_timeout,
            apply_intent_sink: Some(apply_intent.clone()),
        };
        match self.runtime.create(request).await {
            Ok(result) => {
                // Persist the result envelope TOGETHER WITH the REAL
                // post-apply state identity: this is the ownership proof
                // every later Destroy re-verifies at its effect boundary
                // (F07, spec 0004 §5).
                let body = serde_json::json!({
                    "result": result.result_envelope,
                    "state_lineage": result.state_lineage,
                    "state_serial": result.state_serial,
                })
                .to_string();
                self.store
                    .generation_set_result(&generation_id, &body, "sha256:result", now)
                    .await?;
                apply_intent.complete(self.store.as_ref(), now).await?;
                self.store
                    .generation_advance(
                        &generation_id,
                        shaula_core::lifecycle::GenerationState::WaitingOnline,
                        now,
                    )
                    .await?;
                Ok(true)
            }
            Err(shaula_core::ports::TemplateOutcomeError::PlanFailed { .. }) => {
                // No apply admitted; the generation never touched infra.
                self.store
                    .generation_advance(
                        &generation_id,
                        shaula_core::lifecycle::GenerationState::CleanupRequired,
                        now,
                    )
                    .await?;
                Ok(false)
            }
            Err(_) => {
                // Apply may have started: never re-apply; cleanup path.
                self.store
                    .generation_advance(
                        &generation_id,
                        shaula_core::lifecycle::GenerationState::CleanupRequired,
                        now,
                    )
                    .await?;
                Ok(false)
            }
        }
    }
}
