//! Template Runtime port types (provider-neutral IaC seam).

use async_trait::async_trait;

use crate::ports::{BindingsDigest, PlanProvenance, ShaulaInputEnvelope, ShaulaResultEnvelope};
use std::time::Duration;
/// Request for a provider-neutral Create.
#[derive(Clone)]
pub struct TemplateCreateRequest {
    pub workspace_path: std::path::PathBuf,
    pub artifact_dir: std::path::PathBuf,
    /// The artifact digest pinned by the Generation's ATTESTATION tuple
    /// (the fleet revision's resolved (profile, revision, artifact,
    /// attestation) pin). The workspace material is verified against it
    /// before anything applies — material that was swapped in after
    /// activation can never become the trusted baseline (R9-06, spec
    /// 0004 §5).
    pub pinned_artifact_digest: String,
    pub input: ShaulaInputEnvelope,
    pub expected_bindings_digest: BindingsDigest,
    pub managed_shape: Vec<crate::template::ManagedResourceRole>,
    pub environment: Vec<(String, String)>,
    pub timeout: Duration,
    /// Durable sink invoked with the provenance BEFORE any apply spawns;
    // ApplyStarting must be on disk before the child can start
    // (spec 0004 section 6: at-most-once).
    pub apply_intent_sink: Option<std::sync::Arc<dyn ApplyIntentSink>>,
    /// Provider-specific protected bootstrap material. It is deliberately
    /// outside [`ShaulaInputEnvelope`] so the token cannot enter tfvars,
    /// Terraform variables, or operation-log projections.
    pub forgejo_bootstrap: Option<crate::ports::forgejo::ForgejoBootstrapMaterial>,
}
// The executable is deliberately NOT caller-supplied: the runtime adapter
// holds THE single engine authority (its configured binary), so "verify B,
// execute A" is unrepresentable (C3/R10).

/// Definite result of a Create apply.
#[derive(Debug, Clone)]
pub struct TemplateCreateResult {
    pub result_envelope: ShaulaResultEnvelope,
    /// The REAL post-apply state identity from `state pull` (spec 0004
    /// §5: state lineage and serial, never a proxy). The caller persists
    /// it as the generation's ownership proof; Destroy re-verifies it.
    pub state_lineage: String,
    pub state_serial: u64,
    /// Provenance binding computed before the apply; the caller persists
    /// it into the operation ledger.
    pub provenance: PlanProvenance,
}

/// The generation's ORIGINAL persisted state identity (captured at
/// Create). Destroy refuses to run against a state whose lineage is
/// missing or different — a replaced state can never become its own
/// baseline (spec 0004 §5, ownership proof).
#[derive(Debug, Clone)]
pub struct OriginalStateIdentity {
    pub lineage: String,
    pub serial: u64,
    /// Retry attempts may legitimately observe a serial advanced by our
    /// OWN prior partial destroy apply; the first attempt must match the
    /// original serial exactly.
    pub allow_serial_advance: bool,
}

/// Request for a provider-neutral Destroy.
#[derive(Clone)]
pub struct TemplateDestroyRequest {
    /// Archive identity pinned by the durable Generation, including legacy provenance recovery.
    pub pinned_artifact_digest: String,
    pub workspace_path: std::path::PathBuf,
    pub artifact_dir: std::path::PathBuf,
    pub expected_bindings_digest: BindingsDigest,
    pub generation_id: String,
    pub managed_shape: Vec<crate::template::ManagedResourceRole>,
    pub environment: Vec<(String, String)>,
    pub timeout: Duration,
    /// The durable ORIGINAL Create provenance loaded from the ledger: the
    /// destroy must re-verify the SAME engine binary, protected input and
    /// artifact before any apply (spec 0004 §5, exact provenance).
    pub original_provenance: PlanProvenance,
    /// The durable ORIGINAL post-Create state identity; a missing or
    /// mismatched identity refuses the destroy at the effect boundary.
    pub original_state: OriginalStateIdentity,
    /// Durable sink invoked with the provenance BEFORE the destroy apply
    // spawns (`DestroyApplyStarting`).
    pub apply_intent_sink: Option<std::sync::Arc<dyn ApplyIntentSink>>,
}

/// Opaque admission claim returned by [`ApplyIntentSink`] (R5-02): the
/// runtime holds it from the durable apply-start record only until the
/// SPAWN HANDOVER (R6-03) — the moment the apply's process has been
/// spawned under its own process-tree fence and ownership has been
/// handed back to the runtime — never for the child's whole lifetime.
/// Dropping it releases the shared fleet-effect gate. Core never
/// inspects the contents — the concrete guard type is a daemon-internal
/// implementation detail.
pub type ApplyClaim = Box<dyn std::any::Any + Send + Sync>;

/// Persists the apply-start intent durably before a mutating apply
/// spawns, and returns the admission claim held through the in-transaction
/// CAS and the final revalidation, up to the spawn handover. Implemented
/// by the daemon over the operation ledger and the per-fleet effect
/// gates.
#[async_trait]
pub trait ApplyIntentSink: Send + Sync {
    async fn persist_apply_starting(
        &self,
        provenance: &PlanProvenance,
    ) -> Result<ApplyClaim, String>;

    /// Single-use continuation of an admitted Create. Rechecks the Fleet head
    /// and durably marks bootstrap before publishing data or starting a runner.
    /// Legacy callers cannot implicitly acquire this new capability.
    async fn authorize_bootstrap(&self, _: &PlanProvenance) -> Result<ApplyClaim, String> {
        Err("container bootstrap authorization unavailable".into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestroyClassification {
    /// Applied delete-only plan successfully and state is empty.
    Applied,
    /// Bound state was already empty; no apply executed.
    AlreadyEmpty,
}

/// Uncertainty classification for template effects: the caller never infers
/// "did not happen" from a process disappearing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateOutcomeError {
    /// Plan could not be built/admitted; no apply started.
    PlanFailed { phase: String },
    /// Apply failed or its outcome is uncertain; never re-applied by core.
    ExecutionFailed { phase: String },
    /// Workspace/state/proof material missing or corrupt.
    StateUnavailable { phase: String },
}

/// The single platform-neutral IaC seam. Production adapter:
/// `shaula-template`. No platform objects, argv or env maps cross here —
/// only fixed envelopes and bounded policy inputs.
#[async_trait]
pub trait TemplateRuntimePort: Send + Sync {
    /// Prepares the workspace BEFORE any remote effect (R9-07, spec 0004
    /// §6): materialize the pinned artifact and run the LOCKED init.
    /// `init` is long, local and retryable; the JIT token is short-lived,
    /// so it must be minted only after this succeeds. The later
    /// [`Self::create`] re-verifies the prepared material. The default is
    /// a no-op so in-memory mocks stay minimal.
    async fn prepare_create(
        &self,
        _workspace: &std::path::Path,
        _artifact_dir: &std::path::Path,
        _artifact_digest: &str,
        _timeout: Duration,
    ) -> Result<(), TemplateOutcomeError> {
        Ok(())
    }
    /// Returns local evidence that a Forgejo runner resource is not holding
    /// a task. The default is deliberately unknown: remote `idle` status
    /// alone is not a sufficient deletion proof because Forgejo DELETE has
    /// no busy protection.
    async fn forgejo_template_evidence(
        &self,
        _generation_id: &str,
    ) -> crate::ports::forgejo::ForgejoTemplateEvidence {
        crate::ports::forgejo::ForgejoTemplateEvidence::Unknown
    }
    async fn create(
        &self,
        request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError>;
    async fn destroy(
        &self,
        request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError>;
}

impl std::fmt::Debug for TemplateCreateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateCreateRequest")
            .field("workspace_path", &self.workspace_path)
            .field("generation_id", &self.input.generation.id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for TemplateDestroyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateDestroyRequest")
            .field("workspace_path", &self.workspace_path)
            .field("generation_id", &self.generation_id)
            .finish_non_exhaustive()
    }
}
