// ---- Conformance attestation port models (R6-06/R6-08) ----
//
// Split from `store_port` to keep it within the 400-line budget.

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
