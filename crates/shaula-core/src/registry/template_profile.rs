//! Non-secret Template Profile read models and publication inputs.

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateProfileView {
    pub key: String,
    pub incarnation: String,
    pub desired_revision: i64,
    pub active_revision: Option<i64>,
    pub status: String,
    /// Derived only from the admitted artifact manifest.
    pub platform: Option<String>,
    pub bindings_contract: Option<String>,
    /// Bounded presence metadata for sensitive bindings.
    pub bindings_present: bool,
    /// Selection frozen in the Active revision, not the Candidate's capability list.
    /// None means there is no Active revision or its contract cannot be resolved.
    pub runner_backend: Option<crate::fleet::FleetProviderKind>,
}

/// Submission payload for a Template Profile Candidate revision.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateProfilePut {
    pub source_key: Option<String>,
    pub artifact_digest: String,
    pub engine_ref: String,
    pub bindings: serde_json::Value,
    pub fleet_input_policy: serde_json::Value,
}

/// An explicit artifact update that inherits protected bindings from its base.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateProfileUpdate {
    pub source_key: Option<String>,
    pub artifact_digest: String,
    pub engine_ref: String,
    /// None preserves the base revision's bindings verbatim; Some is a
    /// complete desired set resolved per field against the base —
    /// sensitive fields keep on omission or the `null` sentinel and
    /// replace on a new value, non-sensitive fields replace on submission
    /// (spec 0038 §3).
    pub bindings: Option<serde_json::Map<String, serde_json::Value>>,
    /// None preserves the base revision's policy; Some replaces it in full.
    pub fleet_input_policy: Option<serde_json::Map<String, serde_json::Value>>,
}
