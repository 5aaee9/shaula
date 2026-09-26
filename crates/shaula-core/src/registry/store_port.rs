// ---- Persistence ports ----
//
// The daemon sees storage only through these ports. Every composite
// mutation is one atomic unit inside the store: revision + change + audit
// + outbox + idempotency facts commit together or not at all.

use async_trait::async_trait;

use crate::error::CoreResult;
use crate::registry::attestation_port::{
    AttestationCommit, AttestationRecord, AttestationReplayRow,
};
use crate::registry::auth_port::{AuthPromotion, AuthPromotionOutcome, FleetContextAck};
use crate::registry::{Actor, ChangeView, IdempotencyLookup, MutationError};

pub use super::mutation_facts::MutationFacts;

/// Read side of the desired-state store used by admission.
#[async_trait]
pub trait ControlPlaneStore: crate::registry::AuthExecutionStore + Send + Sync {
    async fn demand_with_time(&self, key: &str) -> CoreResult<Option<(i64, Option<i64>)>> {
        Ok(self.demand_get(key).await?.map(|v| (v, None)))
    }
    fn diagnostic_sink(&self) -> Option<std::sync::Arc<dyn crate::diagnostics::DiagnosticSink>> {
        None
    }

    async fn diagnostic_read(
        &self,
        _kind: crate::diagnostics::SubjectKind,
        _key: &str,
        _actor: &Actor,
        _now: i64,
    ) -> crate::diagnostics::DiagnosticsResult {
        Err(crate::diagnostics::DiagnosticsReadError::Unavailable)
    }
    /// Production restores missing cache material from the DB before any read/effect.
    /// Filesystem-only test adapters have no external cache authority to restore.
    async fn ensure_artifact_cached(&self, _digest: &str) -> CoreResult<()> {
        Ok(())
    }
    async fn commit_profile_retirement(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    async fn fleet_get(&self, key: &str) -> CoreResult<Option<FleetHead>>;
    async fn fleet_list(&self, actor: &Actor) -> CoreResult<Vec<(String, i64, String)>>;
    async fn fleet_count(&self) -> CoreResult<usize>;
    async fn fleet_revision_latest(&self, key: &str) -> CoreResult<Option<FleetRevisionRow>>;
    async fn generations_occupancy(&self, fleet_key: &str) -> CoreResult<i64>;
    /// Single generation read for admission-time checks (e.g. the spec
    /// 0028 finalize gate); supervision-wide iteration stays on
    /// `LifecycleStore`. Named `lookup` so callers importing both ports
    /// see no method-resolution ambiguity.
    async fn generation_lookup(
        &self,
        id: &str,
    ) -> CoreResult<Option<crate::registry::GenerationRecord>>;
    async fn capacity_counters(&self, fleet_key: &str) -> CoreResult<(i64, i64)>;
    async fn handoff_get(&self, fleet_key: &str) -> CoreResult<Option<AuthHandoffRow>>;

    async fn template_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>>;
    async fn template_profile_keys(&self) -> CoreResult<Vec<String>>;
    /// Current trusted default catalog entry; historical associations are independent.
    async fn template_source_get(&self, key: &str) -> CoreResult<Option<super::TemplateSource>>;
    async fn template_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<TemplateRevisionRow>>;

    // ---- Shared TemplatePool resources (spec 0037). Defaults keep the
    // in-memory ControlPlaneStore test double source-compatible; the
    // SQLite control plane implements the real ledger. ----

    /// Desired-state head of one shared TemplatePool.
    async fn template_pool_get(
        &self,
        _key: &str,
    ) -> CoreResult<Option<crate::template_pool::TemplatePoolHead>> {
        Ok(None)
    }
    /// Active (non-tombstoned) pools: (key, desired revision, incarnation).
    async fn template_pool_list(&self) -> CoreResult<Vec<(String, i64, String)>> {
        Ok(Vec::new())
    }
    /// Latest committed pool revision with its resolved member rows.
    async fn template_pool_revision_latest(
        &self,
        _key: &str,
    ) -> CoreResult<Option<crate::template_pool::TemplatePoolRevision>> {
        Ok(None)
    }
    /// Commits a pool mutation atomically: head CAS + revision append +
    /// immutable member rows (carried on `facts.template_pool`) +
    /// change/audit/outbox/idempotency. A lost fence race surfaces as
    /// `PreconditionFailed`.
    async fn commit_template_pool_mutation(
        &self,
        _facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
    /// Commits a pool no-op's audit and optional replay atomically after
    /// rechecking its exact live head. No revision/change/outbox is created.
    async fn commit_template_pool_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &Actor,
        idempotency: Option<crate::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Tombstones a pool after the reference check: rejected while any
    /// non-decommissioned fleet revision still references the pool.
    async fn commit_template_pool_delete(
        &self,
        _facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        Ok(Err(MutationError::NotFound))
    }
    async fn template_pool_change_get(&self, _change_id: &str) -> CoreResult<Option<ChangeView>> {
        Ok(None)
    }

    async fn auth_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>>;
    async fn auth_profile_keys(&self) -> CoreResult<Vec<String>>;
    async fn auth_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<AuthRevisionRow>>;
    async fn auth_revision_active(&self, key: &str) -> CoreResult<Option<AuthRevisionRow>>;
    async fn auth_bindings_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Vec<crate::auth_context::AccountBinding>>;
    /// Live Fleet Targets currently desiring this auth profile (active,
    /// blocked and decommissioning phases; tombstones excluded).
    async fn auth_live_dependents(&self, key: &str) -> CoreResult<Vec<AuthDependentTarget>>;
    /// The persisted desired/observed Resolved Auth Context of a fleet.
    async fn fleet_auth_context_get(
        &self,
        fleet_key: &str,
    ) -> CoreResult<Option<FleetAuthContextRow>>;
    /// Marks the context resolution blocked with a sanitized reason and
    /// bounded retry deadline.
    async fn fleet_auth_context_block(
        &self,
        fleet_key: &str,
        reason: &str,
        retry_at: i64,
        now: i64,
    ) -> CoreResult<()>;
    /// v2-aware promotion: the bindings + snapshot commit ATOMICALLY with
    /// the head advance, after the coverage and fingerprint gates (spec
    /// 0011 §4.1/§5.1).
    async fn auth_apply_validation_v2(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
        promotion: Option<AuthPromotion>,
    ) -> CoreResult<AuthPromotionOutcome>;
    /// Protected-memory handoff of credential bytes for the EXACT
    /// revision; used for idempotency comparison, never persisted or
    /// logged. Returns None when the revision does not exist.
    async fn auth_credential_bytes(&self, key: &str, revision: i64) -> CoreResult<Option<Vec<u8>>>;
    /// Protected-memory handoff of a Template Revision's bindings JSON
    /// (schema-sensitive plaintext) plus its stored commitment; used for
    /// idempotency comparison and protected envelope assembly.
    async fn template_protected_bindings(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<(String, String)>>;

    async fn demand_get(&self, fleet_key: &str) -> CoreResult<Option<i64>>;

    /// Durably advances the observed auth tuple after successful
    /// classification. With `context_json` (a v2 exact Resolved Auth
    /// Context), the observed context advances in the SAME transaction
    /// after the durable-pin check; a pin conflict writes NOTHING and
    /// reports [`FleetContextAck::IdentityDrift`].
    async fn handoff_acknowledge(
        &self,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
        context_json: Option<&str>,
        expectation: &crate::registry::AuthHandoffExpectation,
    ) -> CoreResult<FleetContextAck>;
    /// Records a failed proof only while its captured authority is current.
    /// False means stale work was discarded without changing either rollout.
    async fn handoff_mark_blocked(
        &self,
        fleet_key: &str,
        authority: &(String, i64),
        expectation: &crate::registry::AuthHandoffExpectation,
        reason: &str,
        retry_at: i64,
    ) -> CoreResult<bool>;

    async fn fleet_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>>;
    async fn profile_change_get(&self, change_id: &str) -> CoreResult<Option<ChangeView>>;

    /// Looks up a stored idempotent response. `Conflict` means the same
    /// key was reused with a different canonical request hash.
    async fn idempotency_find(
        &self,
        principal: &str,
        operation: &str,
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<IdempotencyLookup>;

    async fn artifact_manifest(&self, digest: &str) -> CoreResult<Option<String>>;
    /// The artifact's declared bindings schema document (JSON Schema) —
    /// the per-field `sensitive` authority for the spec 0038 read
    /// projection and Update merge. `Ok(None)` when the artifact is not
    /// published (or its schema file is absent — callers fail closed to
    /// all-sensitive); any other read failure is an `Err`.
    async fn artifact_bindings_schema(&self, digest: &str) -> CoreResult<Option<String>>;
    /// The artifact's declared parameter schema (JSON Schema document) —
    /// the required/type/alias authority for Fleet inputs (spec 0002 §4.1,
    /// 0004 §3). A read failure is an Err (fail closed), never a silent
    /// "no schema" degradation; an artifact without declared parameters
    /// stores an empty schema document.
    async fn artifact_parameter_schema(&self, digest: &str) -> CoreResult<String>;
    async fn artifact_shape_ok(&self, digest: &str) -> CoreResult<bool>;
    /// SHA-256 of the dependency lock file inside the published artifact.
    async fn artifact_lock_digest(&self, digest: &str) -> CoreResult<Option<String>>;
    /// The dependency lock file TEXT inside the published artifact — the
    /// authority the registry recomputes the attestation provider set
    /// from (spec 0005 §5.1).
    async fn artifact_lock_file(&self, digest: &str) -> CoreResult<Option<String>>;

    /// Commits one effective fleet mutation atomically: revision append,
    /// desired head advance + fence bump, auth handoff desired tuple,
    /// change, audit, outbox and idempotency record.
    // `Err(PreconditionFailed)` maps a lost mutation-fence race (a concurrent
    // DELETE or newer PUT won between admission and commit); the caller
    // maps it onto the HTTP status contract.
    async fn commit_fleet_mutation(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Durably records a NO-OP fleet PUT: an audit entry plus, when the
    /// caller supplied an idempotency key, the stored 200 replay body — so
    /// a retry of the same request replays the original 200 and a key
    /// reuse with a different body conflicts, exactly like a real revision
    /// (spec 0002 §5.2/§5.3). The SAME transaction re-checks the current
    /// head against the no-op's `(incarnation, revision)`; a concurrent
    /// newer PUT or DELETE since the caller's read maps to
    /// `PreconditionFailed` / `Gone` instead of recording a stale no-op.
    async fn commit_fleet_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &Actor,
        idempotency: Option<crate::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Decommission: deletion marker + fence bump + revision + change.
    async fn commit_decommission(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Operator finalization of a Quarantined generation (spec 0028):
    /// one transaction CAS-checks `state = Quarantined`, advances to
    /// `Destroyed`, appends the Finalize operation row, the audit fact and
    /// the optional idempotency record. `Err(NotFound)` for a missing row,
    /// `Err(Conflict)` for a non-quarantined state — never a stale silent
    /// success.
    async fn commit_generation_finalize(
        &self,
        generation_id: &str,
        actor: &Actor,
        reason: &str,
        idempotency: Option<crate::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Commits one Template Candidate revision with its facts. A lost
    /// fence race (a concurrent PUT advanced the head between admission
    /// and commit) surfaces as `Err(MutationError::PreconditionFailed)`,
    /// never as an infrastructure error (R9-02).
    async fn commit_template_revision(
        &self,
        facts: MutationFacts,
        extra: (String, String, String, String),
        source_key: Option<String>,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Durable template-publish NO-OP (R9-02; see commits_noop impl).
    async fn commit_template_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &Actor,
        idempotency: Option<(String, String, String)>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Commits one Auth Candidate revision with credential bytes.
    async fn commit_auth_revision(
        &self,
        facts: MutationFacts,
        credential: AuthRevisionRow,
        secret_bytes: &[u8],
    ) -> CoreResult<Result<(), MutationError>>;
    /// Serializes replay, desired-head and Active-base checks with credential
    /// inheritance and Candidate/change/audit/outbox publication. Replay never
    /// needs the historical credential to remain available.
    async fn commit_auth_policy_update(
        &self,
        facts: MutationFacts,
        base_revision: i64,
        policy_json: String,
    ) -> CoreResult<Result<crate::registry::MutationAccepted, MutationError>>;
    /// Looks up a persisted attestation by its FULL path identity
    /// (Profile, Revision, attestation key). The replay pre-check uses
    /// this to reach the immutable historical record BEFORE the current
    /// engine authority is consulted (R6-07).
    async fn attestation_get(
        &self,
        profile_key: &str,
        revision: i64,
        attestation_key: &str,
    ) -> CoreResult<Option<AttestationReplayRow>>;
    /// Stores immutable conformance evidence in one transaction: replay by
    /// stable identity returns the original record, a conflicting body is
    /// refused. Evidence cannot change activation (spec 0017).
    async fn commit_attestation(
        &self,
        record: AttestationRecord,
        actor: Actor,
        now: i64,
    ) -> CoreResult<Result<AttestationCommit, MutationError>>;
}

pub use super::store_rows::{FleetHead, FleetRevisionRow, ProfileHead, TemplateRevisionRow};

/// One immutable auth revision row lives in [`super::auth_port`].
pub use super::auth_port::{
    AuthDependentTarget, AuthHandoffRow, AuthRevisionRow, FleetAuthContextRow,
};
