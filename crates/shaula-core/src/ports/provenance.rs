//! Durable identity and evidence for one mutating template apply.
use crate::plan::PlanIntent;
use serde::{Deserialize, Serialize};

/// State serial or explicit empty-state sentinel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateLineage {
    Empty,
    /// The applied state identity: Terraform's own lineage UUID and
    /// serial, captured read-only from `state pull` — never a proxy.
    Serial {
        lineage: String,
        serial: u64,
    },
}

/// Provenance binding persisted before any mutating apply (spec 0004 §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanProvenance {
    pub intent: PlanIntent,
    pub saved_plan_digest: String,
    pub engine_kind: String,
    pub engine_version: String,
    pub engine_binary_digest: String,
    pub artifact_digest: String,
    /// Digest of normalized executable files, distinct from the archive identity.
    #[serde(default)]
    pub template_material_digest: String,
    pub protected_input_digest: String,
    /// State serial or explicit empty-state sentinel.
    pub state_lineage: StateLineage,
    pub generation_id: String,
    pub attempt_id: String,
}
