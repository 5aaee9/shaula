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

/// One accepted effective mutation with all its durable facts.
#[derive(Debug, Clone)]
pub struct MutationFacts {
    pub resource_kind: &'static str,
    pub resource_key: String,
    pub incarnation: String,
    pub revision: i64,
    /// Canonical spec JSON (fleet) or empty for decommission markers.
    pub spec_json: String,
    pub template: Option<(String, i64, String, String)>,
    pub auth_desired: Option<(String, i64)>,
    pub inputs_digest: String,
    pub actor: String,
    pub now: i64,
    pub change: ChangeView,
    pub outbox_topic: String,
    pub outbox_payload: String,
    /// Idempotency record to store with the same transaction.
    pub idempotency: Option<(String, String, i32, String)>,
}

/// Read side of the desired-state store used by admission.
#[async_trait]
pub trait ControlPlaneStore: crate::registry::AuthExecutionStore + Send + Sync {
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
    async fn capacity_counters(&self, fleet_key: &str) -> CoreResult<(i64, i64)>;
    async fn handoff_get(&self, fleet_key: &str) -> CoreResult<Option<AuthHandoffRow>>;

    async fn template_profile_get(&self, key: &str) -> CoreResult<Option<ProfileHead>>;
    async fn template_profile_keys(&self) -> CoreResult<Vec<String>>;
    async fn template_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<TemplateRevisionRow>>;

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
        resource_kind: &str,
        resource_key: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> CoreResult<IdempotencyLookup>;

    async fn artifact_manifest(&self, digest: &str) -> CoreResult<Option<String>>;
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
        actor: &str,
        idempotency: Option<crate::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Decommission: deletion marker + fence bump + revision + change.
    async fn commit_decommission(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Commits one Template Candidate revision with its facts. A lost
    /// fence race (a concurrent PUT advanced the head between admission
    /// and commit) surfaces as `Err(MutationError::PreconditionFailed)`,
    /// never as an infrastructure error (R9-02).
    async fn commit_template_revision(
        &self,
        facts: MutationFacts,
        extra: (String, String, String, String),
    ) -> CoreResult<Result<(), MutationError>>;
    /// Durable template-publish NO-OP (R9-02; see commits_noop impl).
    async fn commit_template_noop(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &str,
        idempotency: Option<(String, String)>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>>;
    /// Commits one Auth Candidate revision with credential bytes.
    async fn commit_auth_revision(
        &self,
        facts: MutationFacts,
        credential: AuthRevisionRow,
        secret_bytes: &[u8],
    ) -> CoreResult<Result<(), MutationError>>;
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
        actor: String,
        now: i64,
    ) -> CoreResult<Result<AttestationCommit, MutationError>>;
}

/// Desired-state head of one fleet incarnation.
#[derive(Debug, Clone)]
pub struct FleetHead {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    pub observed_revision: i64,
    pub tombstone: bool,
    pub deletion_marker: bool,
    pub phase: String,
    pub last_condition_reason: Option<String>,
    /// G5: the fence captured BEFORE network validation is the
    /// precondition the acknowledgement must still satisfy.
    pub mutation_fence: i64,
}

/// One immutable fleet revision row.
#[derive(Debug, Clone)]
pub struct FleetRevisionRow {
    pub fleet_key: String,
    pub revision: i64,
    pub spec_json: String,
    pub template_profile_key: Option<String>,
    pub template_revision: Option<i64>,
    pub template_artifact_digest: Option<String>,
    pub template_attestation_id: Option<String>,
    pub auth_desired: (String, i64),
    /// Digest of the admitted normalized template inputs, frozen at
    /// admission — the authority a generation's envelope must match.
    pub inputs_digest: String,
    pub created_at: i64,
}

/// Desired/active head shape shared by profile resources.
#[derive(Debug, Clone)]
pub struct ProfileHead {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    pub active_revision: Option<i64>,
    /// Opaque activation provenance; legacy name retained for stored/wire pins.
    pub active_attestation_id: Option<String>,
    pub status: String,
}

/// One immutable template revision row (bindings plaintext excluded from
/// all read models).
#[derive(Debug, Clone)]
pub struct TemplateRevisionRow {
    pub profile_key: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub engine_ref: String,
    pub platform: Option<String>,
    pub bindings_contract: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub bindings_present: bool,
    pub bindings_digest: Option<String>,
    pub fleet_input_policy_json: Option<String>,
}

/// One immutable auth revision row lives in [`super::auth_port`].
pub use super::auth_port::{
    AuthDependentTarget, AuthHandoffRow, AuthRevisionRow, FleetAuthContextRow,
};
