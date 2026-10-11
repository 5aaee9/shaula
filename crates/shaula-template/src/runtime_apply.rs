//! Saved-plan apply admission, pre-spawn re-verification and spawn handover
//! shared by Create and Destroy. Op-specific checks (Create's empty-state
//! sentinel, Destroy's state-identity re-check) stay at the call sites,
//! between [`TemplateRuntime::verify_pre_spawn`] and
//! [`TemplateRuntime::spawn_apply`].

use std::path::Path;

use shaula_core::ports::{ApplyClaim, ApplyIntentSink, PlanProvenance, TemplateOutcomeError};

use super::{digest_of, exec_err, state_err, TemplateRuntime};
use crate::engine::TerraformFlow;

/// Fixed phase strings reported by one lifecycle operation's apply.
pub(super) struct ApplyPhases {
    apply: &'static str,
    handover: &'static str,
    completion: &'static str,
}

pub(super) const CREATE_PHASES: ApplyPhases = ApplyPhases {
    apply: "create.apply",
    handover: "create.handover",
    completion: "create.completion",
};

pub(super) const DESTROY_PHASES: ApplyPhases = ApplyPhases {
    apply: "destroy.apply",
    handover: "destroy.handover",
    completion: "destroy.completion",
};

/// An admitted apply: the durable intent sink, the pinned provenance and the
/// admission claim (R6-03) held up to the spawn handover.
pub(super) struct AdmittedApply<'a> {
    sink: Option<&'a dyn ApplyIntentSink>,
    provenance: &'a PlanProvenance,
    claim: Option<ApplyClaim>,
    phases: &'static ApplyPhases,
}

impl TemplateRuntime {
    /// Persists the apply-start intent (durable BEFORE the child can spawn,
    /// spec 0004 section 6) and binds the effect to this attempt. The
    /// admission claim (R6-03) is held only through the SPAWN HANDOVER in
    /// [`Self::spawn_apply`], never for the child's lifetime.
    pub(super) async fn admit_apply<'a>(
        sink: Option<&'a dyn ApplyIntentSink>,
        provenance: &'a PlanProvenance,
        phases: &'static ApplyPhases,
    ) -> Result<AdmittedApply<'a>, TemplateOutcomeError> {
        let claim = match sink {
            Some(sink) => Some(
                sink.persist_apply_starting(provenance)
                    .await
                    .map_err(|_| state_err(phases.apply))?,
            ),
            None => None,
        };
        crate::operation_capture::bind_effect(&provenance.attempt_id);
        Ok(AdmittedApply {
            sink,
            provenance,
            claim,
            phases,
        })
    }

    /// Immediately before spawn: re-hash plan, engine binary, the protected
    /// input AND the workspace template files; any mismatch rejects the
    /// apply (spec 0004 sec 5) instead of applying unrecorded material.
    pub(super) async fn verify_pre_spawn(
        &self,
        workspace: &Path,
        admitted: &AdmittedApply<'_>,
    ) -> Result<(), TemplateOutcomeError> {
        let (provenance, phases) = (admitted.provenance, admitted.phases);
        let plan_now =
            std::fs::read(workspace.join("tfplan")).map_err(|_| state_err(phases.apply))?;
        if digest_of(&plan_now) != provenance.saved_plan_digest {
            return Err(exec_err(phases.apply));
        }
        let engine_now = crate::engine::hash_binary_off_thread(&self.engine_executable)
            .await
            .map_err(|_| exec_err(phases.apply))?;
        if engine_now != provenance.engine_binary_digest {
            return Err(exec_err(phases.apply));
        }
        let input_now = std::fs::read(workspace.join("shaula.tfvars.json"))
            .map_err(|_| state_err(phases.apply))?;
        if digest_of(&input_now) != provenance.protected_input_digest {
            return Err(exec_err(phases.apply));
        }
        if self
            .workspace_digest(workspace)
            .await
            .map_err(|_| state_err(phases.apply))?
            != provenance.template_material_digest
        {
            return Err(exec_err(phases.apply));
        }
        Ok(())
    }

    /// SPAWN HANDOVER (R6-03): start_apply_saved_plan returns once the apply
    /// process exists under its own process-tree fence (R6-04). The
    /// admission claim is released EXACTLY here — never earlier (that would
    /// reopen the CAS→spawn race, R5-02) and never held for the child's
    /// lifetime (that would block a waiting DELETE/PUT for the whole
    /// apply). From this point the running child is owned by its fence and
    /// the workspace, not by the admission gate.
    pub(super) async fn spawn_apply(
        &self,
        flow: &TerraformFlow,
        workspace: &Path,
        environment: &[(String, String)],
        admitted: AdmittedApply<'_>,
    ) -> Result<(), TemplateOutcomeError> {
        let AdmittedApply {
            sink,
            provenance,
            claim: admission_claim,
            phases,
        } = admitted;
        self.verify_http_workspace(workspace, true)?;
        let apply_spawn = flow
            .start_apply_saved_plan(workspace, environment)
            .await
            .map_err(|_| exec_err(phases.apply))?;
        if let Some(sink) = sink {
            sink.spawn_handover(provenance)
                .await
                .map_err(|_| exec_err(phases.handover))?;
        }
        drop(admission_claim);
        let apply_result = apply_spawn
            .wait()
            .await
            .and_then(|output| flow.require_success(output, "apply"));
        if let Some(sink) = sink {
            sink.command_ended(provenance)
                .await
                .map_err(|_| exec_err(phases.completion))?;
        }
        apply_result.map_err(|_| exec_err(phases.apply))?;
        Ok(())
    }
}
