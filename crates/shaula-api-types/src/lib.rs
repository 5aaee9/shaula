//! Portable wire contracts. No server, database or runtime dependencies.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
mod history;
mod logs;
mod requests;
mod views;
pub use history::*;
pub use logs::*;
pub use requests::*;
pub use views::*;

/// Secret fields never derive Serialize or Display and Debug is always redacted.
#[derive(Clone, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub principal: Option<Principal>,
    #[serde(default)]
    pub authentication: Option<Authentication>,
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub personal_access_tokens: bool,
    pub token_management_api: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemDetails {
    pub status: Option<u16>,
    pub code: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Authentication {
    pub kind: String,
    pub token_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub issuer: String,
    pub subject: String,
}

/// Actual normal mutation casing, including replayable NoOp responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalMutation {
    #[serde(rename = "changeId")]
    pub change_id: String,
    pub state: String,
    pub revision: i64,
    #[serde(rename = "noOp", default, skip_serializing_if = "std::ops::Not::not")]
    pub no_op: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub id: String,
    #[serde(alias = "resourceKind")]
    pub resource_kind: String,
    #[serde(alias = "resourceKey")]
    pub resource_key: String,
    pub revision: i64,
    pub kind: String,
    pub state: String,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizeReceipt {
    pub etag: String,
    pub change: Change,
    pub no_op: bool,
}

/// Preserve future response fields and all original JSON number spellings.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Document(pub Box<serde_json::value::RawValue>);
impl Document {
    pub fn from_serializable(value: impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self(serde_json::value::to_raw_value(&value)?))
    }
    pub fn parse(raw: String) -> Result<Self, serde_json::Error> {
        serde_json::from_str(&raw)
    }
    pub fn raw(&self) -> &str {
        self.0.get()
    }
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(self.raw())
    }
}
impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Document([REDACTED])")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fleet {
    pub key: String,
    pub spec: Document,
    pub metadata: RevisionMetadata,
    pub resolved: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionMetadata {
    pub incarnation: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    #[serde(flatten)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessToken {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub effective_scopes: Vec<String>,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    pub state: String,
    pub revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueToken {
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Clone)]
pub enum TokenIssue {
    Issued {
        metadata: AccessToken,
        secret: Secret,
    },
    RecoveredWithoutSecret(AccessToken),
}
impl TokenIssue {
    pub fn metadata(&self) -> &AccessToken {
        match self {
            Self::Issued { metadata, .. } | Self::RecoveredWithoutSecret(metadata) => metadata,
        }
    }
}
impl std::fmt::Debug for TokenIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenIssue")
            .field("metadata", self.metadata())
            .finish_non_exhaustive()
    }
}
