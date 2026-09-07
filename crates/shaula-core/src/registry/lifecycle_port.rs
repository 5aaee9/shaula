//! The `LifecycleStore` port: generations, operations, sessions,
//! ownership and convergence writes (spec 0001 section 10, 0004 section 6),
//! split to keep files within 400 lines (AGENTS.md).

use async_trait::async_trait;

use crate::error::CoreResult;

/// Persistence surface for the per-fleet supervisor: sessions, demand,
/// generations and scale set ownership. All runner rows are Fleet-Key
/// namespaced so identical message/job suffixes across Fleets can never
/// cross-dedupe, cross-wake or cross-recover.
#[async_trait]
pub trait LifecycleStore: Send + Sync {
    async fn session_epoch(&self, fleet_key: &str) -> CoreResult<Option<i64>>;
    async fn session_install(
        &self,
        fleet_key: &str,
        session_id: &str,
        scale_set_id: i64,
        now: i64,
    ) -> CoreResult<i64>;
    async fn demand_snapshot(
        &self,
        fleet_key: &str,
        total_assigned_jobs: i64,
        now: i64,
    ) -> CoreResult<()>;

    async fn scale_set_get(&self, fleet_key: &str) -> CoreResult<Option<ScaleSetRow>>;
    async fn scale_set_upsert(&self, row: ScaleSetRow) -> CoreResult<()>;

    async fn generation_insert(&self, record: GenerationRecord) -> CoreResult<()>;
    async fn generation_get(&self, id: &str) -> CoreResult<Option<GenerationRecord>>;
    async fn generations_for_fleet(&self, fleet_key: &str) -> CoreResult<Vec<GenerationRecord>>;
    async fn generation_advance(
        &self,
        id: &str,
        next: crate::lifecycle::GenerationState,
        now: i64,
    ) -> CoreResult<()>;
    async fn generation_set_github_runner(
        &self,
        id: &str,
        runner_id: i64,
        now: i64,
    ) -> CoreResult<()>;
    /// Durable JIT-intent marker (R9-07, spec 0004 §6): records the JIT
    /// phase BEFORE the remote JIT request is issued, so a lost response
    /// leaves recovery evidence instead of an invisible half-effect.
    async fn generation_set_jit_phase(&self, id: &str, phase: &str, now: i64) -> CoreResult<()>;
    /// Persists a NON-apply operation row (e.g. the durable `JitStarting`
    /// intent with its full request context, R9-07). Apply operations go
    /// through [`Self::operation_record_apply_starting`], which carries
    /// the fleet fence.
    async fn operation_insert(&self, insert: crate::registry::OperationInsert) -> CoreResult<()>;
    async fn generation_set_result(
        &self,
        id: &str,
        result_json: &str,
        digest: &str,
        now: i64,
    ) -> CoreResult<()>;
    /// The generation's ORIGINAL post-Create state identity
    /// `(lineage, serial)` — the ownership proof a later Destroy
    /// re-verifies at the effect boundary (spec 0004 §5). `None` means
    /// the Create never recorded one (corrupt; quarantine).
    async fn generation_state_identity(&self, id: &str) -> CoreResult<Option<(String, u64)>>;
    /// Whether any Destroy operation was ever recorded for this
    /// generation: a retry attempt may see a serial advanced by our own
    /// prior partial apply, the first attempt must match exactly.
    async fn generation_destroy_attempted(&self, id: &str) -> CoreResult<bool>;
    /// Persists the durable `ApplyStarting` / `DestroyApplyStarting`
    /// operation record with full provenance before a mutating apply
    /// spawns (spec 0004 §6, at-most-once). The fence lives INSIDE this
    /// single transaction: a Create is refused when the fleet head moved
    /// or the deletion marker is set; a Destroy is refused only when the
    /// generation row is missing — decommission must never block cleanup
    /// (spec 0002 §8).
    async fn operation_record_apply_starting(
        &self,
        generation_id: &str,
        kind: &str,
        provenance_json: &str,
        saved_plan_path: &str,
        saved_plan_digest: &str,
        now: i64,
    ) -> CoreResult<()>;
    /// The durable ORIGINAL provenance of the generation's Create — the
    /// exact pin a later Destroy must re-verify against (spec 0004 §5).
    async fn operation_original_provenance(
        &self,
        generation_id: &str,
    ) -> CoreResult<Option<crate::ports::PlanProvenance>>;
    /// Marks one operation with a new state after its effect resolves
    /// (apply finished, apply failed, …) and bumps its attempt counter
    /// (spec 0004 §6).
    async fn operation_update_state(&self, id: &str, state: &str, now: i64) -> CoreResult<()>;
    /// Operations of a generation still in a non-terminal state.
    async fn operations_open_for_generation(
        &self,
        generation_id: &str,
    ) -> CoreResult<Vec<OperationRow>>;
    /// Observes convergence on the fleet: the observed revision, the
    /// lifecycle phase and the human-readable condition reason.
    async fn fleet_set_observed(
        &self,
        key: &str,
        observed_revision: i64,
        phase: &str,
        reason: Option<&str>,
        now: i64,
    ) -> CoreResult<()>;
    /// Terminal state after a completed decommission: the fleet can never
    /// admit or execute anything again (spec 0002 §8).
    async fn fleet_set_tombstone(&self, key: &str, now: i64) -> CoreResult<()>;
    /// Change-state transition with retry bookkeeping (spec 0002 §6).
    async fn change_update(
        &self,
        id: &str,
        state: &str,
        reason: Option<&str>,
        next_retry_at: Option<i64>,
        now: i64,
    ) -> CoreResult<()>;
    /// Profile-change-state transition with retry bookkeeping.
    async fn profile_change_update(
        &self,
        id: &str,
        state: &str,
        reason: Option<&str>,
        next_retry_at: Option<i64>,
        now: i64,
    ) -> CoreResult<()>;
    /// Materializes the message-queue endpoint of an established session.
    async fn session_set_queue(
        &self,
        fleet_key: &str,
        message_queue_url: &str,
        queue_token: &str,
        now: i64,
    ) -> CoreResult<()>;
    /// The fleet-head guard at the apply-spawn boundary: the current
    /// `(desired_revision, deleting)` - where `deleting` covers the
    /// deletion marker and the tombstone. The apply-intent sink must
    /// verify the generation's own revision against this INSIDE the
    /// durable persist call, so a DELETE/newer PUT that landed during
    /// init/plan/JIT can never let a stale Create spawn (spec 0001
    /// section 10.2.7, 0002 mutation fence).
    async fn fleet_head_guard(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<crate::registry::FleetHeadGuard>>;
}

/// The fleet-head guard tuple used at the apply-spawn boundary:
/// `(desired_revision, deleting)` where deleting covers the deletion
/// marker and the tombstone.
pub type FleetHeadGuard = (i64, bool);

/// Persisted scale set ownership row.
#[derive(Debug, Clone)]
pub struct ScaleSetRow {
    pub fleet_key: String,
    pub scale_set_id: Option<i64>,
    pub name: String,
    pub runner_group: String,
    pub fingerprint: String,
    pub state: String,
    pub attempt_id: Option<String>,
    pub now: i64,
}

/// One non-terminal runner-operation row (effect bookkeeping).
#[derive(Debug, Clone)]
pub struct OperationRow {
    pub id: String,
    pub generation_id: String,
    pub kind: String,
    pub state: String,
    pub attempts: i64,
    pub saved_plan_digest: Option<String>,
}

/// Persisted runner generation record.
#[derive(Debug, Clone)]
pub struct GenerationRecord {
    pub id: String,
    pub fleet_key: String,
    pub runner_name: String,
    pub generation_name: String,
    pub fleet_revision: i64,
    pub template_profile_key: String,
    pub template_revision: i64,
    pub template_artifact_digest: String,
    pub attestation_id: String,
    pub inputs_digest: String,
    pub state: crate::lifecycle::GenerationState,
    pub github_runner_id: Option<i64>,
    pub workspace_path: String,
    pub created_at: i64,
    pub updated_at: i64,
}
