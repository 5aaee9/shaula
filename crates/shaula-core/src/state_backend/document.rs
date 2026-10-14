use std::{collections::BTreeMap, fmt};

use serde::Deserialize;
use zeroize::Zeroizing;

use super::{StateError, StateResult, MAX_STATE_BYTES};
use crate::template::ManagedResourceRole;

/// Raw state bytes are preserved exactly, including on an idempotent replay.
/// Debug/Display/serialization must never accidentally expose provider secrets.
pub struct StateDocument {
    bytes: Zeroizing<Vec<u8>>,
    lineage: String,
    serial: i64,
    managed_empty: bool,
    managed_counts: BTreeMap<String, u64>,
}

#[derive(Deserialize)]
struct Header {
    version: u32,
    lineage: String,
    serial: i64,
    resources: Vec<Resource>,
    outputs: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct Resource {
    mode: String,
    #[serde(rename = "type")]
    resource_type: String,
    name: String,
    provider: String,
    instances: Vec<serde_json::Map<String, serde_json::Value>>,
}

impl StateDocument {
    /// Accept raw Terraform v4, never `show -json` or a partial header. This
    /// validates the backend envelope, not provider-specific attribute values.
    /// Header duplicates and malformed instance containers fail closed.
    pub fn parse(bytes: Vec<u8>) -> StateResult<Self> {
        let bytes = Zeroizing::new(bytes);
        if bytes.len() > MAX_STATE_BYTES {
            return Err(StateError::TooLarge);
        }
        let header: Header = serde_json::from_slice(&bytes).map_err(|_| StateError::Invalid)?;
        if header.version != 4
            || header.serial < 0
            || header.lineage.is_empty()
            || header.lineage.len() > 256
            || header.lineage.chars().any(char::is_control)
            || header.resources.iter().any(|r| {
                !matches!(r.mode.as_str(), "managed" | "data")
                    || r.resource_type.is_empty()
                    || r.name.is_empty()
                    || r.provider.is_empty()
            })
        {
            return Err(StateError::Invalid);
        }
        if header.outputs.values().any(|v| !v.is_object()) {
            return Err(StateError::Invalid);
        }
        let managed_empty = header
            .resources
            .iter()
            .all(|r| r.mode != "managed" || r.instances.is_empty());
        let mut managed_counts = BTreeMap::new();
        for resource in header.resources.iter().filter(|r| r.mode == "managed") {
            *managed_counts
                .entry(resource.resource_type.clone())
                .or_insert(0) += resource.instances.len() as u64;
        }
        Ok(Self {
            bytes,
            lineage: header.lineage,
            serial: header.serial,
            managed_empty,
            managed_counts,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn lineage(&self) -> &str {
        &self.lineage
    }

    pub fn serial(&self) -> i64 {
        self.serial
    }

    /// Empty known state alone is NOT proof of Destroy or of no side effects.
    pub fn managed_empty(&self) -> bool {
        self.managed_empty
    }

    /// Managed instance objects per Terraform type. Tainted and deposed objects
    /// count: each is a remote resource the state still tracks.
    pub fn managed_counts(&self) -> &BTreeMap<String, u64> {
        &self.managed_counts
    }

    /// Whether this state tracks exactly the managed resources a Create of
    /// `shape` admits (spec 0010 §7). Create plan admission allows no other
    /// managed instance, so a failed Create whose state records the whole
    /// shape cannot have left an untracked resource behind; a partial shape
    /// may have (a provider error before the object was saved).
    pub fn records_whole_shape(&self, shape: &[ManagedResourceRole]) -> bool {
        let mut expected = BTreeMap::new();
        for role in shape {
            *expected.entry(role.terraform_type.as_str()).or_insert(0) += role.exact_count;
        }
        !expected.is_empty()
            && self
                .managed_counts
                .iter()
                .filter(|(_, count)| **count > 0)
                .count()
                == expected.len()
            && expected
                .iter()
                .all(|(kind, count)| self.managed_counts.get(*kind).copied().unwrap_or(0) == *count)
    }
}

impl fmt::Debug for StateDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StateDocument(REDACTED)")
    }
}
