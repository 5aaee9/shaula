//! The Template Runtime: single platform-neutral seam implementing the
//! core-owned `TemplateRuntimePort`. It hides artifact materialization,
//! protected files, saved-plan admission, apply classification and the
//! state-empty proof behind fixed envelopes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::plan::{admit_create_plan, parse_plan};
use shaula_core::ports::{
    DestroyClassification, PlanProvenance, StateLineage, TemplateCreateRequest,
    TemplateCreateResult, TemplateDestroyRequest, TemplateOutcomeError, TemplateRuntimePort,
};
use shaula_core::template::ShaulaResultEnvelope;

use crate::engine::TerraformFlow;
use crate::workspace;

fn plan_err(phase: &str) -> TemplateOutcomeError {
    TemplateOutcomeError::PlanFailed {
        phase: phase.to_string(),
    }
}

fn exec_err(phase: &str) -> TemplateOutcomeError {
    TemplateOutcomeError::ExecutionFailed {
        phase: phase.to_string(),
    }
}

fn state_err(phase: &str) -> TemplateOutcomeError {
    TemplateOutcomeError::StateUnavailable {
        phase: phase.to_string(),
    }
}

fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

#[path = "runtime_input.rs"]
mod input;
use input::write_protected_input;

/// Copies the materialized artifact root into the workspace (the working
/// copy the engine mutates), leaving the store pristine.
pub(crate) fn materialize_into_workspace(
    artifact_dir: &PathBuf,
    workspace: &PathBuf,
) -> CoreResult<()> {
    crate::workspace_materialize::copy_recursive(artifact_dir, workspace)
}

/// The runtime handle shared across supervisor tasks.
#[derive(Clone)]
pub struct TemplateRuntime {
    engine_executable: PathBuf,
    http_backend: Option<crate::http_backend::HttpBackendConfig>,
}

impl TemplateRuntime {
    /// Legacy local-state / static-validation mode. Never implicitly migrate
    /// an existing Generation when the new worker backend is available.
    pub fn new(engine_executable: PathBuf) -> Self {
        Self {
            engine_executable,
            http_backend: None,
        }
    }

    /// New, explicitly admitted HTTP-state Generation only. The caller owns
    /// worker/GitHub gates and must select the corresponding runtime attestation.
    pub fn with_http_backend(
        engine_executable: PathBuf,
        backend: crate::http_backend::HttpBackendConfig,
    ) -> Self {
        Self {
            engine_executable,
            http_backend: Some(backend),
        }
    }

    async fn open_flow(&self, timeout: Duration) -> Result<TerraformFlow, TemplateOutcomeError> {
        TerraformFlow::open(self.engine_executable.clone(), timeout)
            .await
            .map_err(|e| match e.code {
                ReasonCode::TemplatePlanFailed => plan_err("create.plan"),
                _ => exec_err("create.apply"),
            })
    }
}

#[async_trait]
impl TemplateRuntimePort for TemplateRuntime {
    /// R9-07 (spec 0004 §6): materialize + LOCKED init before any remote
    /// effect — init is long, local and retryable; the JIT token minted
    /// afterwards is short-lived.
    async fn prepare_create(
        &self,
        workspace: &Path,
        artifact_dir: &Path,
        artifact_digest: &str,
        timeout: Duration,
    ) -> Result<(), TemplateOutcomeError> {
        self.prepare_create_flow(workspace, artifact_dir, artifact_digest, timeout)
            .await
    }

    async fn create(
        &self,
        mut request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        if self.http_backend.is_some() && request.apply_intent_sink.is_none() {
            return Err(state_err("create.authorization"));
        }
        self.configure_environment(&request.input.generation.id, &mut request.environment)?;
        let workspace = &request.workspace_path;
        let expected_material = crate::artifact_integrity::material_digest(
            &request.artifact_dir,
            &request.pinned_artifact_digest,
        )
        .map_err(|_| state_err("create.materialize"))?;
        // A supervisor that ran prepare_create before minting the JIT has
        // already materialized + initialized (its `.terraform` marker
        // proves it); re-doing both would burn JIT token lifetime for
        // nothing. Direct callers (tests, future paths) get the full
        // materialize + init here.
        let prepared = workspace.join(".terraform").is_dir();
        if !prepared {
            self.require_fresh_http_workspace(workspace)?;
            materialize_into_workspace(&request.artifact_dir, workspace)
                .map_err(|_| state_err("create.materialize"))?;
            if let Some(backend) = &self.http_backend {
                backend
                    .install(workspace)
                    .map_err(|_| state_err("create.backend"))?;
            }
        }
        self.verify_http_workspace(workspace, prepared)?;

        if self
            .workspace_digest(workspace)
            .map_err(|_| state_err("create.materialize"))?
            != expected_material
        {
            return Err(state_err("create.materialize"));
        }
        let flow = self.open_flow(request.timeout).await?;
        if !prepared {
            self.initialize_workspace(&flow, workspace, &request.environment)
                .await?;
        }

        let input_digest = write_protected_input(workspace, &request.input)
            .map_err(|_| state_err("create.plan"))?;

        flow.plan(workspace, &request.environment, false)
            .await
            .map_err(|_| plan_err("create.plan"))?;
        let plan_json = flow
            .show_plan_json(workspace, &request.environment)
            .await
            .map_err(|_| plan_err("create.plan"))?;
        let parsed = parse_plan(&plan_json).map_err(|_| plan_err("create.plan"))?;
        // Fail closed when the workspace already holds managed state:
        // Create requires a provably empty prior state.
        let prior_state = flow
            .state_list(workspace, &request.environment)
            .await
            .map_err(|_| state_err("create.plan"))?;
        admit_create_plan(&parsed, prior_state.is_empty(), &request.managed_shape)
            .map_err(|_| plan_err("create.plan"))?;
        let plan_bytes =
            std::fs::read(workspace.join("tfplan")).map_err(|_| state_err("create.plan"))?;
        // The provenance pins the ACTUAL executed material: the workspace's
        // own template files (identical to the artifact source by
        // materialize, but verified where they run), not just the store
        // directory (spec 0004 §5). R9-06: the material must ALSO equal
        // the ATTESTED artifact the Generation froze — replacement of the
        // artifact after activation can never become the trusted
        // baseline.
        let workspace_template_digest = self
            .workspace_digest(workspace)
            .map_err(|_| state_err("create.plan"))?;
        if workspace_template_digest != expected_material {
            return Err(state_err("create.plan"));
        }
        let provenance = PlanProvenance {
            intent: shaula_core::plan::PlanIntent::Create,
            saved_plan_digest: digest_of(&plan_bytes),
            engine_kind: "terraform".to_string(),
            engine_version: flow.version.clone(),
            engine_binary_digest: flow.binary_digest.clone(),
            artifact_digest: request.pinned_artifact_digest.clone(),
            template_material_digest: workspace_template_digest,
            protected_input_digest: input_digest,
            state_lineage: if prior_state.is_empty() {
                StateLineage::Empty
            } else {
                return Err(state_err("create.plan"));
            },
            generation_id: request.input.generation.id.clone(),
            attempt_id: shaula_core::auth::new_attempt_id(),
        };

        // ApplyStarting must be durable BEFORE the child can spawn
        // (spec 0004 section 6, at-most-once). The admission claim
        // (R6-03) is held only through the SPAWN HANDOVER below —
        // released the moment start_apply_saved_plan returns, never
        // held for the child's lifetime.
        let _admission_claim = match &request.apply_intent_sink {
            Some(sink) => Some(
                sink.persist_apply_starting(&provenance)
                    .await
                    .map_err(|_| state_err("create.apply"))?,
            ),
            None => None,
        };

        // Immediately before spawn: re-hash plan, engine binary, the
        // protected input AND the workspace template files; any mismatch
        // rejects the apply (spec 0004 sec 5).
        let plan_now =
            std::fs::read(workspace.join("tfplan")).map_err(|_| state_err("create.apply"))?;
        if digest_of(&plan_now) != provenance.saved_plan_digest {
            return Err(exec_err("create.apply"));
        }
        let engine_now = crate::engine::hash_binary(&self.engine_executable)
            .map_err(|_| exec_err("create.apply"))?;
        if engine_now != provenance.engine_binary_digest {
            return Err(exec_err("create.apply"));
        }
        let input_now = std::fs::read(workspace.join("shaula.tfvars.json"))
            .map_err(|_| state_err("create.apply"))?;
        if digest_of(&input_now) != provenance.protected_input_digest {
            return Err(exec_err("create.apply"));
        }
        if self
            .workspace_digest(workspace)
            .map_err(|_| state_err("create.apply"))?
            != provenance.template_material_digest
        {
            return Err(exec_err("create.apply"));
        }
        // Pre-spawn EMPTY-state sentinel re-check (F07, spec 0004 §5):
        // the prior state was proven empty before planning; it must STILL
        // be empty immediately before the apply spawns. A state that
        // appeared in between refuses the apply.
        let prior_now = flow
            .state_list(workspace, &request.environment)
            .await
            .map_err(|_| state_err("create.apply"))?;
        if !prior_now.is_empty() {
            return Err(state_err("create.apply"));
        }

        // SPAWN HANDOVER (R6-03): start_apply_saved_plan returns once
        // the apply process exists under its own process-tree fence
        // (R6-04). The admission claim is released EXACTLY here — never
        // earlier (that would reopen the CAS→spawn race, R5-02) and
        // never held for the child's lifetime (that would block a
        // waiting DELETE/PUT for the whole apply). From this point the
        // running child is owned by its fence and the workspace, not by
        // the admission gate.
        self.verify_http_workspace(workspace, true)?;
        let apply_spawn = flow
            .start_apply_saved_plan(workspace, &request.environment)
            .await
            .map_err(|_| exec_err("create.apply"))?;
        drop(_admission_claim);
        apply_spawn
            .wait()
            .await
            .and_then(|output| flow.require_success(output, "apply"))
            .map_err(|_| exec_err("create.apply"))?;

        self.verify_http_workspace(workspace, true)?;
        let outputs = flow
            .output_json(workspace, &request.environment)
            .await
            .map_err(|_| exec_err("create.apply"))?;
        let result_value =
            crate::engine::extract_shaula_result(&outputs).map_err(|_| exec_err("create.apply"))?;
        let envelope: ShaulaResultEnvelope =
            serde_json::from_value(result_value.clone()).map_err(|_| exec_err("create.apply"))?;
        envelope
            .validate_against(
                &request.input.generation.id,
                &request.expected_bindings_digest,
                &request.managed_shape,
            )
            .map_err(|_| exec_err("create.apply"))?;

        // Post-apply state identity: managed state must exist and the
        // REAL lineage+serial are captured for the ledger — they become
        // the generation's ownership proof at every later Destroy
        // (spec 0004 §5).
        let snapshot = flow
            .state_pull_snapshot(workspace, &request.environment)
            .await
            .map_err(|_| state_err("state.verify"))?;
        if snapshot.managed.is_empty() {
            return Err(exec_err("create.apply"));
        }

        Ok(TemplateCreateResult {
            result_envelope: envelope,
            state_lineage: snapshot.lineage,
            state_serial: snapshot.serial,
            provenance,
        })
    }

    async fn destroy(
        &self,
        request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.destroy_flow(request).await
    }
}
#[path = "runtime_backend.rs"]
mod backend;
#[path = "runtime_destroy.rs"]
mod destroy_flow;

/// Convenience wrapper for constructing the workspace before a Create.
pub fn prepare_workspace(
    work_root: &std::path::Path,
    fleet_key: &str,
    generation_id: &str,
) -> CoreResult<PathBuf> {
    let path = workspace::generation_workspace(work_root, fleet_key, generation_id)?;
    workspace::create_workspace(&path)?;
    Ok(path)
}

/// Removes a workspace after terminal Destroy with empty-state proof.
pub fn cleanup_workspace(
    work_root: &std::path::Path,
    fleet_key: &str,
    generation_id: &str,
) -> CoreResult<()> {
    let path = workspace::generation_workspace(work_root, fleet_key, generation_id)?;
    workspace::remove_workspace(&path)
}

/// Shared handle type used across the daemon.
pub type SharedTemplateRuntime = Arc<TemplateRuntime>;
