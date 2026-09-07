//! The protected `shaula` input and `shaula_result` output envelopes
//! (spec 0004 §3), split to keep the host file within 400 lines.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ReasonCode};
use crate::template::{BindingsDigest, ManagedResourceRole};

/// Generation identity written into the protected `shaula` input envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationIdentity {
    pub fleet_key: String,
    pub scale_set_id: i64,
    pub id: String,
    /// Display name registered at GitHub; short and human-scannable, NOT
    /// a platform resource name.
    pub runner_name: String,
    /// Collision-proof generation-scoped platform resource name
    /// (Kubernetes metadata.name semantics: ≤63 chars), persisted by
    /// Shaula before any external effect.
    pub generation_name: String,
}

/// System-side protected input envelope; exactly one top-level variable
/// `shaula` in the tfvars document.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ShaulaInputEnvelope {
    pub contract_version: u32,
    pub generation: GenerationIdentity,
    /// Write-only value; serialized into the protected tfvars only.
    pub jit_config: String,
    pub bindings_digest: BindingsDigest,
    #[serde(default)]
    pub bindings: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub parameters: serde_json::Map<String, serde_json::Value>,
}

// Type-bound redaction (spec 0007 §3.2/§5): the JIT and the bindings carry
// credentials (kubeconfig), so plain-text Debug is unrepresentable. The
// tfvars serializer below is the single sanctioned materialization.
impl std::fmt::Debug for ShaulaInputEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShaulaInputEnvelope")
            .field("contract_version", &self.contract_version)
            .field("generation", &self.generation)
            .field("jit_config", &"[REDACTED]")
            .field("bindings_digest", &self.bindings_digest)
            .field("bindings", &"[REDACTED]")
            .field("parameters", &self.parameters)
            .finish()
    }
}

pub const INPUT_CONTRACT_VERSION: u32 = 1;

impl ShaulaInputEnvelope {
    pub fn new(
        generation: GenerationIdentity,
        jit_config: String,
        bindings_digest: BindingsDigest,
    ) -> Self {
        Self {
            contract_version: INPUT_CONTRACT_VERSION,
            generation,
            jit_config,
            bindings_digest,
            bindings: serde_json::Map::new(),
            parameters: serde_json::Map::new(),
        }
    }

    /// Renders the protected tfvars JSON document whose sole top-level
    /// variable is `shaula`.
    pub fn to_tfvars(&self) -> CoreResult<String> {
        let doc = serde_json::json!({ "shaula": self });
        serde_json::to_string(&doc).map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))
    }
}

/// One managed resource echo in the `shaula_result` envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultResource {
    pub role: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<String>,
}

/// Fixed provider-neutral output envelope; stored as protected opaque
/// evidence after validation, never interpreted per platform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaulaResultEnvelope {
    pub contract_version: u32,
    pub generation_id: String,
    pub bindings_digest: BindingsDigest,
    #[serde(default)]
    pub resources: Vec<ResultResource>,
}

pub const RESULT_CONTRACT_VERSION: u32 = 1;
/// Upper bound on resources echoed in one result envelope.
pub const MAX_RESULT_RESOURCES: usize = 8;
const MAX_OPAQUE_ID_LEN: usize = 512;

impl ShaulaResultEnvelope {
    /// Verifies contract version, echoed generation id, exact
    /// `bindings_digest`, declared roles/cardinality and bounded sizes.
    pub fn validate_against(
        &self,
        expected_generation_id: &str,
        expected_bindings: &BindingsDigest,
        expected_shape: &[ManagedResourceRole],
    ) -> CoreResult<()> {
        if self.contract_version != RESULT_CONTRACT_VERSION {
            return Err(CoreError::new(
                ReasonCode::TemplateExecutionFailed,
                "unsupported shaula_result contract version",
            ));
        }
        if self.generation_id != expected_generation_id {
            return Err(CoreError::new(
                ReasonCode::TemplateExecutionFailed,
                "shaula_result generation id mismatch",
            ));
        }
        if &self.bindings_digest != expected_bindings {
            return Err(CoreError::new(
                ReasonCode::TemplateExecutionFailed,
                "shaula_result bindings_digest mismatch",
            ));
        }
        if self.resources.len() != expected_shape.len() {
            return Err(CoreError::new(
                ReasonCode::TemplateExecutionFailed,
                "shaula_result resource cardinality mismatch",
            ));
        }
        for role in &self.resources {
            if role.id.is_empty() || role.id.len() > MAX_OPAQUE_ID_LEN {
                return Err(CoreError::new(
                    ReasonCode::TemplateExecutionFailed,
                    "shaula_result resource id out of bounds",
                ));
            }
            if !expected_shape.iter().any(|s| s.role == role.role) {
                return Err(CoreError::new(
                    ReasonCode::TemplateExecutionFailed,
                    "shaula_result unexpected resource role",
                ));
            }
            if let Some(inc) = &role.incarnation {
                if inc.len() > MAX_OPAQUE_ID_LEN {
                    return Err(CoreError::new(
                        ReasonCode::TemplateExecutionFailed,
                        "shaula_result incarnation out of bounds",
                    ));
                }
            }
        }
        // Each declared role must appear exactly the declared number of times.
        for shape in expected_shape {
            let count = self
                .resources
                .iter()
                .filter(|r| r.role == shape.role)
                .count();
            if count != shape.exact_count as usize {
                return Err(CoreError::new(
                    ReasonCode::TemplateExecutionFailed,
                    "shaula_result role cardinality mismatch",
                ));
            }
        }
        Ok(())
    }
}

/// Parameter object for recording static-validation results.
#[derive(Debug, Clone)]
pub struct TemplateValidationRecord {
    pub key: String,
    pub revision: i64,
    pub ready: bool,
    pub platform: String,
    pub bindings_contract: String,
    pub manifest_json: String,
    pub lock_digest: String,
    pub reason: Option<String>,
}

/// Parameter object for storing one conformance attestation.
#[derive(Debug, Clone)]
pub struct AttestationInsert {
    pub id: String,
    pub key: String,
    pub revision: i64,
    pub subject_json: String,
    pub subject_digest: String,
    pub result: String,
    pub evidence_digest: Option<String>,
    pub suite_name: Option<String>,
    pub suite_version: Option<String>,
    pub completed_at: i64,
    /// R10-05: the durable subject-verification verdict.
    pub subject_verified: bool,
}
