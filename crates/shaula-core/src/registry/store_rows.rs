//! Admission read models, re-exported through the existing storage interface.

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
    pub template_pool: Vec<crate::template_pool::ResolvedTemplatePoolMember>,
    /// Shared-pool routing context frozen at admission (spec 0037 §4);
    /// `template_pool` is hydrated from the pool revision members for
    /// pool-referencing fleets.
    pub template_pool_ref: Option<crate::template_pool::FleetPoolRef>,
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

/// One immutable template revision row. `bindings_json` is the
/// protected-memory seam for the schema-driven projection (spec 0038);
/// it is never serialized into a response, log or audit record.
#[derive(Debug, Clone)]
pub struct TemplateRevisionRow {
    pub profile_key: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub engine_ref: String,
    pub source_key: Option<String>,
    pub platform: Option<String>,
    pub bindings_contract: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub bindings_present: bool,
    pub bindings_json: Option<String>,
    pub bindings_digest: Option<String>,
    pub fleet_input_policy_json: Option<String>,
}
