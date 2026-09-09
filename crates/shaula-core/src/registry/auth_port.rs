// ---- Auth Profile persistence row models (spec 0011) ----
//
// Split from `store_port` to keep both files within the 400-line budget.
// Credential bytes never appear in any read model here.

use crate::error::CoreResult;

/// Revision-attributed read state. Unsupported historical revisions carry
/// no policy or bindings; active authority is never merged with a Candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthRevisionState {
    pub revision: i64,
    pub state: String,
    pub reason: Option<String>,
    pub binding_health: Vec<AuthBindingHealth>,
    /// Only 2 is supported; other stored formats are historical metadata.
    pub schema_version: i64,
    /// v2: the App id in the stored representation.
    pub app_id: Option<String>,
    /// v2: the frozen Target policy.
    pub target_policy: Option<crate::auth_policy::TargetPolicy>,
    /// v2: the frozen Account Bindings (empty until promotion).
    pub bindings: Vec<crate::auth_context::AccountBinding>,
}

/// Current route health, separate from immutable revision bindings.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AuthBindingHealth {
    pub account_id: i64,
    pub installation_id: i64,
    pub state: String,
    pub reason: Option<String>,
    pub checked_at_ms: Option<i64>,
    pub valid_until_ms: Option<i64>,
    pub affected_fleets: Vec<String>,
}

/// A live fleet target currently desiring this auth profile (spec 0011
/// §6): the pre-publication live-Fleet impact surface for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthLiveFleet {
    pub fleet_key: String,
    pub phase: String,
    /// The fleet's GitHub target; None only if the stored spec is
    /// unreadable (never silently treated as covered).
    pub target: Option<crate::github::GitHubTarget>,
}

/// Non-secret Auth Profile summary; only `credential_present` metadata.
/// The ACTIVE revision is reported separately from the DESIRED Candidate
/// so a Pending/Rejected upgrade never hides the still-effective
/// authorization (spec 0011 §6).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthProfileView {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    pub active_revision: Option<i64>,
    pub status: String,
    pub kind: Option<crate::auth::AuthKind>,
    pub credential_present: bool,
    /// The desired head's format version.
    pub schema_version: i64,
    /// The App id of the desired head (v2 heads).
    pub app_id: Option<String>,
    /// The ACTIVE revision's state, attributed to its own revision.
    pub active: Option<AuthRevisionState>,
    /// The DESIRED Candidate's state when it differs from the active
    /// head.
    pub desired: Option<AuthRevisionState>,
    /// Live fleet targets desiring this profile, for the pre-publication
    /// live-Fleet impact preview (spec 0011 §6).
    pub live_fleets: Vec<AuthLiveFleet>,
}

/// One immutable auth revision row. Credential bytes never leave the
/// GitHub Access Module seam; this read model carries only metadata.
#[derive(Debug, Clone)]
pub struct AuthRevisionRow {
    pub profile_key: String,
    pub revision: i64,
    pub state: String,
    pub reason: Option<String>,
    pub kind: String,
    pub app_id: Option<String>,
    /// Only 2 is executable; other stored formats are historical metadata.
    pub schema_version: i64,
    /// Canonical TargetPolicy JSON for schema version 2.
    pub policy_json: Option<String>,
    /// The promoted revision's validation snapshot (identities +
    /// checked fleet state) — the continuity authority for rotations.
    pub validation_snapshot_json: Option<String>,
}

impl AuthRevisionRow {
    /// The stored Target policy of a v2 revision. Legacy rows have none;
    /// a stored policy that no longer parses is an internal fault.
    pub fn target_policy(&self) -> CoreResult<Option<crate::auth_policy::TargetPolicy>> {
        match &self.policy_json {
            None => Ok(None),
            Some(json) => serde_json::from_str(json).map(Some).map_err(|e| {
                crate::error::CoreError::new(
                    crate::error::ReasonCode::Internal,
                    format!("policy invalid: {e}"),
                )
            }),
        }
    }
}

/// A live dependent Fleet Target of an Auth Profile, captured for the
/// policy-coverage gate (spec 0011 §5.1). Includes fleets still
/// EXECUTING on this profile through their observed handoff reference
/// during a cross-profile replacement, so a shrink can never strand
/// effects that still run against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthDependentTarget {
    pub fleet_key: String,
    pub fleet_phase: String,
    pub target_json: String,
    /// Fleet incarnation/revision + mutation fence at read time: the
    /// promotion transaction revalidates them before the head advance.
    pub incarnation: String,
    pub revision: i64,
    pub fence: i64,
    /// Exact contexts retained by observed execution, sessions or cleanup.
    pub retained_contexts: Vec<crate::auth_context::ResolvedAuthContext>,
    /// Includes legacy references, whose execution has no v2 context.
    pub retained_refs: Vec<(String, i64)>,
}

/// Authority captured before remote handoff proof. Both values must still
/// match in the acknowledgement transaction; neither is optional evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthHandoffExpectation {
    pub mutation_fence: i64,
    /// Missing context is refused at the acknowledgement boundary.
    pub desired_context_json: Option<String>,
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

/// Desired/observed exact Resolved Auth Context of one fleet (spec 0011
/// §5.2). The JSON payloads are non-secret route metadata; both carry the
/// full ref tuple so a ref match with a drifted context stays detectable.
#[derive(Debug, Clone)]
pub struct FleetAuthContextRow {
    pub fleet_key: String,
    pub desired: Option<(String, i64)>,
    pub desired_context_json: Option<String>,
    pub observed: Option<(String, i64)>,
    pub observed_context_json: Option<String>,
    /// Pending / Observed / Blocked.
    pub state: String,
    pub reason: Option<String>,
}

/// Outcome of the observed-context CAS at the port boundary (spec 0011
/// §5.2 step 4). `IdentityDrift` means the verified context contradicts a
/// durable identity pin — never overwritten, caller must block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetContextAck {
    /// Observed ref + context advanced.
    Acknowledged,
    /// The stored desired ref moved on; the caller lost the race.
    Stale,
    /// The verified context contradicts a durable identity pin.
    IdentityDrift,
}

/// The durable-pin rule (spec 0011 §4.2/§5.1): the TARGET's numeric
/// identity — account id, organization id, repository id + owner id — is
/// immutable per fleet target, so a same-name rebuild or ownership
/// transfer can never silently rewrite it. Canonical display names may
/// change; ids may not. The INSTALLATION ROUTE is deliberately separate:
/// it is replaced ONLY by an explicitly promoted newer revision (a
/// reinstall accepted through a new Candidate + Handoff, spec 0011
/// §5.1); a same-revision acknowledgement must never observe a
/// different route (background reinstall), while a newer revision must.
pub fn auth_context_pins_agree(
    observed: &crate::auth_context::ResolvedAuthContext,
    verified: &crate::auth_context::ResolvedAuthContext,
) -> bool {
    let id_matches = |pinned: Option<i64>, verified: Option<i64>| match (pinned, verified) {
        (Some(pinned), Some(verified)) => pinned == verified,
        // A verified absence never erases an existing pin.
        (Some(_), None) => false,
        (None, _) => true,
    };
    // An explicit route replacement is a NEWER revision of the SAME
    // profile (sanctioned reinstall) OR a VALIDATED cross-profile
    // replacement — revision counters of different profiles are
    // unrelated, so profile identity, not ordering, decides (F7/R8).
    let explicit_route_change =
        verified.profile_key != observed.profile_key || verified.revision > observed.revision;
    id_matches(observed.organization_id, verified.organization_id)
        && id_matches(observed.repository_id, verified.repository_id)
        && id_matches(observed.repository_owner_id, verified.repository_owner_id)
        && observed.account_id == verified.account_id
        && observed.account_kind == verified.account_kind
        && (observed.installation_id == verified.installation_id || explicit_route_change)
}

/// Promotion facts the validator supplies for a v2 Candidate: the
/// verified Account Bindings and the validation snapshot (dependent-set
/// fingerprint + checked fleet identities), committed atomically with the
/// head advance (spec 0011 §4.1 step 7).
#[derive(Debug, Clone)]
pub struct AuthPromotion {
    pub bindings: Vec<crate::auth_context::AccountBinding>,
    pub snapshot_json: String,
}

/// Outcome of one Auth Candidate promotion transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPromotionOutcome {
    /// The Candidate is Active and the prior head was replaced.
    Promoted,
    /// The Candidate is Rejected; the prior active head is untouched.
    Rejected,
    /// The dependent set changed under validation: the Candidate returns
    /// to validation (spec 0011 §5.1) with the prior head untouched.
    Restaged,
}

/// One live fleet whose identity/fence the validator checked. The
/// promotion transaction revalidates EVERY member against the CURRENT
/// fleet state, so any concurrent mutation restages the Candidate (spec
/// 0011 §4.1 step 6, §5.1).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthCheckedFleet {
    pub key: String,
    pub incarnation: String,
    pub revision: i64,
    pub fence: i64,
}

/// One account's PROVEN numeric identity at validation time. Compared
/// against the PREVIOUS revision's snapshot on rotation: account ids are
/// immutable per login (a reused login is a different account, spec 0011
/// §5.1), installation routes may be replaced explicitly, and exact
/// repository selectors carry durable repository/owner ids.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthIdentityProof {
    pub login: String,
    pub account_id: i64,
    pub installation_id: i64,
    #[serde(default)]
    pub repositories: Vec<AuthRepoProof>,
}

/// Numeric identity of one exact repository selector, proven through the
/// installation credential. A same-name rebuild with different ids is
/// `TargetIdentityChanged`, never a silent rebind.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthRepoProof {
    pub owner: String,
    pub repository: String,
    pub repository_id: i64,
    pub owner_id: i64,
}

/// The dependent-set state a validator checked, persisted with the
/// promoted v2 revision (spec 0011 §4.1 step 6). Non-secret. The
/// promotion transaction revalidates every member before the head
/// advance.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthValidationSnapshot {
    /// The Candidate ref this snapshot was produced for.
    pub candidate: (String, i64),
    /// Fingerprint of the (fleet_key, target) dependency set.
    pub dependent_set: String,
    /// The live fleets whose identity the validator checked.
    #[serde(default)]
    pub checked_fleets: Vec<AuthCheckedFleet>,
    /// The proven numeric identities behind the Candidate's selectors.
    #[serde(default)]
    pub identities: Vec<AuthIdentityProof>,
}

/// THE canonical dependent-set fingerprint: length-prefixed hash over the
/// sorted `fleet_key + target` pairs. Recomputed inside the promotion
/// transaction as the dependent-set CAS (spec 0011 §5.1) — the single
/// authority, so validator and store can never diverge on the encoding.
pub fn auth_dependent_set_fingerprint(dependents: &[AuthDependentTarget]) -> String {
    let mut parts: Vec<String> = dependents
        .iter()
        .map(|d| {
            let mut contexts: Vec<_> = d
                .retained_contexts
                .iter()
                .map(|c| {
                    let fields = [
                        c.profile_key.clone(),
                        c.revision.to_string(),
                        c.github_host.clone(),
                        c.app_id.clone(),
                        c.account_id.to_string(),
                        format!("{:?}", c.account_kind),
                        c.login.clone(),
                        c.installation_id.to_string(),
                        c.target.config_url(),
                        format!("{:?}", c.organization_id),
                        format!("{:?}", c.repository_id),
                        format!("{:?}", c.repository_owner_id),
                    ];
                    let bytes: Vec<_> = fields.iter().map(String::as_bytes).collect();
                    crate::auth::request_hash_parts(&bytes)
                })
                .collect();
            contexts.sort();
            let mut refs: Vec<_> = d
                .retained_refs
                .iter()
                .map(|(key, revision)| {
                    crate::auth::request_hash_parts(&[
                        key.as_bytes(),
                        revision.to_string().as_bytes(),
                    ])
                })
                .collect();
            refs.sort();
            let mut fields = vec![
                d.fleet_key.clone(),
                d.target_json.clone(),
                contexts.len().to_string(),
            ];
            fields.extend(contexts);
            fields.extend(refs);
            let bytes: Vec<_> = fields.iter().map(String::as_bytes).collect();
            crate::auth::request_hash_parts(&bytes)
        })
        .collect();
    parts.sort();
    let byte_parts: Vec<&[u8]> = parts.iter().map(String::as_bytes).collect();
    crate::auth::request_hash_parts(&byte_parts)
}

#[path = "auth_execution_port.rs"]
mod execution;
pub use execution::{AuthExecutionStore, AuthRouteObservation};
