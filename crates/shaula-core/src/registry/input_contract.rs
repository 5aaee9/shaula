//! Bounded, non-secret projection of one exact Template Revision's input authority.

use serde::{Deserialize, Serialize};

/// An approved complete JSON value encoded without a browser Number round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputOption {
    pub value_json: String,
}

/// One settable top-level input and its finite approved values.
#[derive(Debug, Clone, Serialize)]
pub struct InputField {
    pub key: String,
    pub label: String,
    pub description: String,
    pub required: bool,
    pub options: Vec<InputOption>,
}

/// Root enum values must remain whole presets, never independently combined fields.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum InputContractProjection {
    Fields { fields: Vec<InputField> },
    Presets { presets: Vec<InputOption> },
}

/// Read model bound to immutable revision materials; no binding values or artifacts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInputContract {
    pub version: u8,
    pub profile_key: String,
    pub incarnation: String,
    pub revision: i64,
    pub artifact_digest: String,
    #[serde(flatten)]
    pub projection: InputContractProjection,
}

/// Expected read failures; storage failures remain CoreError rather than empty forms.
#[derive(Debug, Clone)]
pub enum InputContractReadError {
    Forbidden,
    NotFound,
    Unavailable { reason: String },
}
