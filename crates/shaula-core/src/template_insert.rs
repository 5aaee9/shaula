//! Parameter object for creating a Template Candidate revision.

use serde::{Deserialize, Serialize};

/// Parameter object for creating a Template Candidate revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRevisionInsert {
    pub key: String,
    pub incarnation: String,
    pub revision: i64,
    pub artifact_digest: String,
    pub engine_ref: String,
    pub source_key: Option<String>,
    pub bindings_json: Option<String>,
    pub bindings_digest: Option<String>,
    pub fleet_input_policy_json: Option<String>,
}
