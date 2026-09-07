// ---- Persistence ports ----
//
// The daemon sees storage only through these ports. Every composite
// mutation is one atomic unit inside the store: revision + change + audit
// + outbox + idempotency facts commit together or not at all.

use async_trait::async_trait;

use crate::error::CoreResult;
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
pub trait ControlPlaneStore: Send + Sync {
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
    /// Applies GitHub identity/access validation outcome for an Auth
    /// Candidate (staged activation; false keeps prior active).
    async fn auth_apply_validation(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
    ) -> CoreResult<()>;
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

    /// Durably advances the observed auth tuple to the full desired tuple
    /// after successful classification.
    async fn handoff_acknowledge(
        &self,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
    ) -> CoreResult<()>;
    /// Marks the handoff blocked with a sanitized reason and retry deadline.
    async fn handoff_mark_blocked(
        &self,
        fleet_key: &str,
        reason: &str,
        retry_at: i64,
    ) -> CoreResult<()>;

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
    /// Stores an immutable attestation and optionally activates it, in
    /// ONE transaction (spec 0005 §5): replay by stable identity returns
    /// the original record, a conflicting body is refused, the
    /// `Ready -> Active` freeze happens atomically with the insert, and
    /// a refused activation still leaves the evidence durable+audited.
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
    pub bindings_present: bool,
    pub bindings_digest: Option<String>,
    pub fleet_input_policy_json: Option<String>,
}

/// One immutable auth revision row. Credential bytes never leave the
/// GitHub Access Module seam; this read model carries only metadata.
#[derive(Debug, Clone)]
pub struct AuthRevisionRow {
    pub profile_key: String,
    pub revision: i64,
    pub kind: String,
    pub app_id: Option<String>,
    pub installation_id: Option<i64>,
    pub pat_principal: Option<String>,
    pub allowlist_json: String,
}

/// Persisted auth handoff state for one fleet.
#[derive(Debug, Clone)]
pub struct AuthHandoffRow {
    pub fleet_key: String,
    pub desired: (String, i64),
    pub observed: Option<(String, i64)>,
    pub state: String,
    pub cleanup_only: bool,
    pub blocked_reason: Option<String>,
    pub retry_at: Option<i64>,
}

/// Derives the STABLE persisted identity of an attestation from its
/// FULL path scope (R6-06): Profile, Revision and the URI attestation
/// key. The single hashing authority — the service derives record ids
/// with it and the store looks replays up with it, so the two can never
/// diverge. The part encoding is length-prefixed (R7-05), so distinct
/// (profile, revision, key) tuples can never collide on the same digest
/// even around embedded separator bytes.
pub fn attestation_record_id(profile_key: &str, revision: i64, attestation_key: &str) -> String {
    crate::auth::request_hash_parts(&[
        profile_key.as_bytes(),
        revision.to_string().as_bytes(),
        attestation_key.as_bytes(),
    ])
}

/// One immutable conformance attestation to store (R6-06). The STABLE
/// resource identity is the FULL path scope — Profile, Revision and the
/// attestation key from the request URI — hashed into `id`, so
/// unrelated profiles (or a new revision) reusing the same URI key never
/// collide. An exact replay of the same identity and canonical body
/// returns the original record; the same identity with a different body
/// conflicts.
#[derive(Debug, Clone)]
pub struct AttestationRecord {
    /// sha256 over (profile_key, revision, attestation_key).
    pub id: String,
    /// The attestation key exactly as it appeared in the request URI —
    /// for the audit fact, never persisted as the record identity.
    pub attestation_key: String,
    pub profile_key: String,
    pub revision: i64,
    pub subject_json: String,
    pub subject_digest: String,
    /// The result AS CLAIMED by the submitter ("passed"/"failed"/...).
    /// The verification verdict lives in the audit fact, not here.
    pub result: String,
    pub evidence_digest: Option<String>,
    pub suite: (String, String),
    pub completed_at: i64,
    /// Whether the submitted subject matched the recomputed authority.
    /// A mismatched attestation is stored and audited but can never
    /// activate (spec 0005 §5, R6-08).
    pub subject_verified: bool,
    /// When set, atomically freeze this attestation as the active one.
    pub activate: bool,
}

/// The persisted attestation row as seen by the replay pre-check
/// (R6-07): the service compares the canonical request members against
/// it BEFORE touching the current engine authority.
#[derive(Debug, Clone)]
pub struct AttestationReplayRow {
    pub profile_key: String,
    pub revision: i64,
    pub subject_json: String,
    pub result: String,
    pub evidence_digest: Option<String>,
    pub suite_name: Option<String>,
    pub suite_version: Option<String>,
    pub completed_at: i64,
    /// R10-05: whether the subject matched the recomputed authority.
    pub subject_verified: bool,
}

/// Outcome of one attestation commit (spec 0005 §5 replay semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationCommit {
    /// The attestation was newly recorded and (being eligible) froze
    /// itself as the active one.
    Created,
    /// The attestation was recorded and audited but its activation was
    /// refused — stale (no longer desired), candidate not Ready, or the
    /// revision's attestation is already frozen. "Cannot activate" is
    /// never "cannot record" (R6-08).
    RecordedNotActivated,
    /// An exact replay of the same attestation identity: the ORIGINAL
    /// record stands, nothing was re-activated or overwritten.
    Replayed,
}
