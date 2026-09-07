//! The typed canonical attestation subject (R7-04, spec 0005 §5), split
//! from `registry.rs` to stay within the 400-line limit (AGENTS.md).

/// Every member the Registry recomputes from authority is a named field
/// and NOTHING else is accepted: `deny_unknown_fields` turns submissions
/// carrying excluded payload (raw test output, bindings, credentials)
/// into 422 rejections at the request boundary, and the durable record
/// can only ever contain this bounded, sanitized shape.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationSubject {
    pub artifact_digest: String,
    pub dependency_lock_digest: String,
    pub bindings_digest: String,
    pub runtime_policy_digest: String,
    pub platform: String,
    pub bindings_contract: String,
    pub engine: AttestationEngineSubject,
    pub runner_image_digests: Vec<String>,
    pub providers: Vec<AttestationProviderSubject>,
    pub suite: AttestationSuiteSubject,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationEngineSubject {
    pub kind: String,
    pub required_version: String,
    /// The version the conformance run ACTUALLY executed.
    pub version: String,
    pub binary_digest: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationProviderSubject {
    pub source: String,
    pub version: String,
    pub checksums_digest: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationSuiteSubject {
    pub name: String,
    pub version: String,
}
