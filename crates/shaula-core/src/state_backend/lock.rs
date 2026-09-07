use std::fmt;

use serde::{Deserialize, Serialize};

use super::{StateError, StateResult, MAX_LOCK_BYTES};

#[derive(Clone, PartialEq, Eq)]
pub struct LockId(String);

impl LockId {
    pub fn parse(value: String) -> StateResult<Self> {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(StateError::Invalid);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LockId(REDACTED)")
    }
}

/// Terraform's standard LockInfo. Metadata is bounded, retained for an
/// authenticated conflict response, and NEVER used to authenticate a caller.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct WireLock {
    #[serde(rename = "ID")]
    id: String,
    #[serde(default)]
    operation: String,
    #[serde(default)]
    info: String,
    #[serde(default)]
    who: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    created: String,
    #[serde(default)]
    path: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LockInfo {
    id: LockId,
    wire: WireLock,
}

impl LockInfo {
    pub fn parse(bytes: &[u8]) -> StateResult<Self> {
        if bytes.len() > MAX_LOCK_BYTES {
            return Err(StateError::TooLarge);
        }
        let wire: WireLock = serde_json::from_slice(bytes).map_err(|_| StateError::Invalid)?;
        let id = LockId::parse(wire.id.clone())?;
        let value = Self { id, wire };
        // Escaping can expand a conflict response; bound the canonical form too.
        if value.to_bytes()?.len() > MAX_LOCK_BYTES {
            return Err(StateError::TooLarge);
        }
        Ok(value)
    }

    pub fn id(&self) -> &LockId {
        &self.id
    }

    pub fn to_bytes(&self) -> StateResult<Vec<u8>> {
        serde_json::to_vec(&self.wire).map_err(|_| StateError::Invalid)
    }
}

impl fmt::Debug for LockInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LockInfo(REDACTED)")
    }
}
