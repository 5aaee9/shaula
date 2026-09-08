//! Destroy flow, split from `runtime.rs` to keep every file within the
//! 400-line limit (AGENTS.md). It runs as an inherent method so the
//! `TemplateRuntimePort` trait impl in `runtime.rs` stays a thin seam.

use shaula_core::plan::{admit_destroy_plan_json, destroy_empty_state_short_circuit};
use shaula_core::ports::{
    DestroyClassification, PlanProvenance, StateLineage, TemplateDestroyRequest,
    TemplateOutcomeError,
};

use super::{digest_of, exec_err, plan_err, state_err, TemplateRuntime};

impl TemplateRuntime {
    pub(super) async fn destroy_flow(
        &self,
        mut request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        if self.http_backend.is_some() && request.apply_intent_sink.is_none() {
            return Err(state_err("destroy.authorization"));
        }
        self.configure_environment(&request.generation_id, &mut request.environment)?;
        let workspace = &request.workspace_path;
        self.verify_http_workspace(workspace, true)?;
        let flow = self.open_flow(request.timeout).await?;
        let original = &request.original_provenance;

        // EXACT provenance against the durable Create pin (spec 0004 §5):
        // the workspace input, the WORKSPACE template material, and the
        // engine binary must be the SAME objects the Create admitted and
        // applied. Re-hashing today's material against today's own baseline
        // would let a swapped input or a replaced engine self-approve.
        let input_now = std::fs::read(workspace.join("shaula.tfvars.json"))
            .map_err(|_| state_err("destroy.plan"))?;
        if digest_of(&input_now) != original.protected_input_digest {
            return Err(state_err("destroy.plan"));
        }
        let workspace_now = self
            .workspace_digest(workspace)
            .map_err(|_| state_err("destroy.plan"))?;
        let expected_material = crate::artifact_integrity::material_digest(
            &request.artifact_dir,
            &request.pinned_artifact_digest,
        )
        .map_err(|_| state_err("destroy.plan"))?;
        if !crate::artifact_integrity::provenance_matches(
            original,
            &request.pinned_artifact_digest,
            &expected_material,
        ) || workspace_now != expected_material
        {
            return Err(state_err("destroy.plan"));
        }
        if flow.binary_digest != original.engine_binary_digest {
            return Err(exec_err("destroy.plan"));
        }

        let snapshot = flow
            .state_pull_snapshot(workspace, &request.environment)
            .await
            .map_err(|_| state_err("state.verify"))?;
        // ORIGINAL-identity ownership proof (F07, spec 0004 §5/§6.214):
        // the state handed to us must still be the state THIS generation
        // created — same lineage, and the serial of the first attempt
        // must match the persisted post-Create serial exactly. A retry
        // may see a serial advanced by our OWN prior partial apply; a
        // serial that went BACKWARDS is a restored/rolled-back state and
        // is refused. A missing lineage could never pass the parser, so
        // a refusal here always means "state replaced", never "state
        // absent".
        let identity = &request.original_state;
        if snapshot.lineage != identity.lineage {
            return Err(state_err("state.verify"));
        }
        let serial_ok = if identity.allow_serial_advance {
            snapshot.serial >= identity.serial
        } else {
            snapshot.serial == identity.serial
        };
        if !serial_ok {
            return Err(state_err("state.verify"));
        }
        if destroy_empty_state_short_circuit(&snapshot.managed) {
            return Ok(DestroyClassification::AlreadyEmpty);
        }

        flow.plan(workspace, &request.environment, true)
            .await
            .map_err(|_| plan_err("destroy.plan"))?;
        let plan_json = flow
            .show_plan_json(workspace, &request.environment)
            .await
            .map_err(|_| plan_err("destroy.plan"))?;
        admit_destroy_plan_json(&plan_json, &snapshot.managed, &request.managed_shape)
            .map_err(|_| plan_err("destroy.plan"))?;

        // DestroyApplyStarting must be durable before the child spawns
        // (spec 0004 section 6). The saved plan is this run's own baseline;
        // the lineage is the REAL state identity from `state pull`.
        let plan_bytes =
            std::fs::read(workspace.join("tfplan")).map_err(|_| state_err("destroy.plan"))?;
        let provenance = PlanProvenance {
            intent: shaula_core::plan::PlanIntent::Destroy,
            saved_plan_digest: digest_of(&plan_bytes),
            engine_kind: "terraform".to_string(),
            engine_version: flow.version.clone(),
            engine_binary_digest: flow.binary_digest.clone(),
            artifact_digest: request.pinned_artifact_digest.clone(),
            template_material_digest: workspace_now,
            protected_input_digest: original.protected_input_digest.clone(),
            state_lineage: StateLineage::Serial {
                lineage: snapshot.lineage.clone(),
                serial: snapshot.serial,
            },
            generation_id: request.generation_id.clone(),
            attempt_id: shaula_core::auth::new_attempt_id(),
        };
        // The admission claim (R6-03) is held only through the SPAWN
        // HANDOVER below — released the moment start_apply_saved_plan
        // returns, never held for the child's lifetime.
        let _admission_claim = match &request.apply_intent_sink {
            Some(sink) => Some(
                sink.persist_apply_starting(&provenance)
                    .await
                    .map_err(|_| state_err("destroy.apply"))?,
            ),
            None => None,
        };

        // Immediately before spawn: re-verify EVERY member of the
        // provenance — plan, engine binary, protected input, workspace
        // template material AND the state identity (lineage + serial); a
        // swap between admission and spawn rejects the apply (spec 0004
        // sec 5) instead of applying unrecorded material.
        let plan_now =
            std::fs::read(workspace.join("tfplan")).map_err(|_| state_err("destroy.apply"))?;
        if digest_of(&plan_now) != provenance.saved_plan_digest {
            return Err(exec_err("destroy.apply"));
        }
        let engine_now = crate::engine::hash_binary(&self.engine_executable)
            .map_err(|_| exec_err("destroy.apply"))?;
        if engine_now != provenance.engine_binary_digest {
            return Err(exec_err("destroy.apply"));
        }
        let input_now = std::fs::read(workspace.join("shaula.tfvars.json"))
            .map_err(|_| state_err("destroy.apply"))?;
        if digest_of(&input_now) != provenance.protected_input_digest {
            return Err(exec_err("destroy.apply"));
        }
        if self
            .workspace_digest(workspace)
            .map_err(|_| state_err("destroy.apply"))?
            != provenance.template_material_digest
        {
            return Err(exec_err("destroy.apply"));
        }
        let snapshot_now = flow
            .state_pull_snapshot(workspace, &request.environment)
            .await
            .map_err(|_| state_err("state.verify"))?;
        if snapshot_now.serial != snapshot.serial
            || snapshot_now.lineage != snapshot.lineage
            || snapshot_now.managed != snapshot.managed
        {
            return Err(exec_err("destroy.apply"));
        }

        // SPAWN HANDOVER (R6-03): the admission claim is released once
        // the destroy apply process is spawned under its tree fence —
        // after the final verification, never before it, and never held
        // for the child's whole lifetime.
        self.verify_http_workspace(workspace, true)?;
        let apply_spawn = flow
            .start_apply_saved_plan(workspace, &request.environment)
            .await
            .map_err(|_| exec_err("destroy.apply"))?;
        drop(_admission_claim);
        apply_spawn
            .wait()
            .await
            .and_then(|output| flow.require_success(output, "apply"))
            .map_err(|_| exec_err("destroy.apply"))?;

        self.verify_http_workspace(workspace, true)?;
        let after = flow
            .state_pull_snapshot(workspace, &request.environment)
            .await
            .map_err(|_| state_err("state.verify"))?;
        if !after.managed.is_empty() {
            return Err(exec_err("destroy.apply"));
        }
        Ok(DestroyClassification::Applied)
    }
}
