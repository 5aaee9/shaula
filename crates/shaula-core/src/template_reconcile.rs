//! Optional read-only failed-Create reconciliation contract (ARD-0044).

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ReasonCode};
use crate::template::ManagedResourceRole;

pub const DISCOVER_CONTRACT_VERSION: u32 = 1;
pub const MAX_RECONCILE_SETTLE_DELAY_SECS: u64 = 600;
const MAX_IMPORT_ID_LEN: usize = 512;

/// Optional manifest declaration enabling read-only remote discovery and
/// targeted import for failed Creates whose bound state is incomplete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestReconcile {
    /// Relative directory inside the same artifact holding the read-only
    /// discovery module (e.g. `reconcile`).
    pub discover_module: String,
    /// Minimum seconds after entering `Destroying` before running `discover`,
    /// covering eventually-consistent provider views such as Proxmox RRD.
    #[serde(default)]
    pub settle_delay_secs: u64,
}

impl ManifestReconcile {
    pub fn validate(&self) -> CoreResult<()> {
        let dir = self.discover_module.trim();
        if dir.is_empty()
            || dir != self.discover_module
            || dir.len() > 128
            || dir.starts_with('/')
            || dir.starts_with('\\')
            || dir.contains('\\')
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "reconcile.discover_module must be a clean relative path",
            ));
        }
        let segments: Vec<&str> = dir.split('/').collect();
        if segments.is_empty()
            || segments.iter().any(|segment| {
                segment.is_empty()
                    || *segment == "."
                    || *segment == ".."
                    || *segment == ".terraform"
                    || *segment == "schemas"
                    || !segment
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            })
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "reconcile.discover_module contains disallowed path segments",
            ));
        }
        if self.settle_delay_secs > MAX_RECONCILE_SETTLE_DELAY_SECS {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                format!(
                    "reconcile.settle_delay_secs exceeds {MAX_RECONCILE_SETTLE_DELAY_SECS} seconds"
                ),
            ));
        }
        Ok(())
    }
}

/// Structured output emitted by `output "shaula_discover"` in the read-only
/// discovery module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverOutput {
    pub contract_version: u32,
    pub generation_id: String,
    pub roles: Vec<DiscoverRoleResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverRoleResult {
    pub role: String,
    pub resources: Vec<DiscoveredResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredResource {
    pub address: String,
    #[serde(default)]
    pub import_id: Option<String>,
}

impl DiscoverOutput {
    /// Enforces exact 1-to-1 coverage of `managed_resource_shape`, ownership
    /// binding to `expected_generation_id`, declared resource types, and safe
    /// CLI-inject-free `import_id` values.
    pub fn validate(
        &self,
        expected_generation_id: &str,
        shape: &[ManagedResourceRole],
    ) -> CoreResult<()> {
        if self.contract_version != DISCOVER_CONTRACT_VERSION {
            return Err(CoreError::new(
                ReasonCode::TemplatePlanFailed,
                "unsupported shaula_discover contract_version",
            ));
        }
        if self.generation_id != expected_generation_id {
            return Err(CoreError::new(
                ReasonCode::TemplatePlanFailed,
                "shaula_discover generation_id mismatch",
            ));
        }
        if self.roles.len() != shape.len() {
            return Err(CoreError::new(
                ReasonCode::TemplatePlanFailed,
                "shaula_discover roles must match managed_resource_shape exactly",
            ));
        }
        let mut seen_roles = Vec::with_capacity(self.roles.len());
        let mut seen_addresses = Vec::new();
        for entry in &self.roles {
            if seen_roles.contains(&entry.role.as_str()) {
                return Err(CoreError::new(
                    ReasonCode::TemplatePlanFailed,
                    "shaula_discover contains duplicate role",
                ));
            }
            seen_roles.push(entry.role.as_str());
            let Some(declared) = shape.iter().find(|r| r.role == entry.role) else {
                return Err(CoreError::new(
                    ReasonCode::TemplatePlanFailed,
                    format!("shaula_discover references undeclared role {}", entry.role),
                ));
            };
            if entry.resources.len() as u64 > declared.exact_count {
                return Err(CoreError::new(
                    ReasonCode::TemplatePlanFailed,
                    format!(
                        "shaula_discover found {} resources for role {}, exceeding exact_count {}",
                        entry.resources.len(),
                        entry.role,
                        declared.exact_count
                    ),
                ));
            }
            let prefix = format!("{}.", declared.terraform_type);
            for resource in &entry.resources {
                validate_address(&resource.address, &prefix)?;
                if seen_addresses.contains(&resource.address.as_str()) {
                    return Err(CoreError::new(
                        ReasonCode::TemplatePlanFailed,
                        "shaula_discover contains duplicate resource address",
                    ));
                }
                seen_addresses.push(resource.address.as_str());
                if let Some(import_id) = &resource.import_id {
                    validate_import_id(import_id)?;
                }
            }
        }
        Ok(())
    }

    pub fn all_resources(&self) -> Vec<&DiscoveredResource> {
        self.roles
            .iter()
            .flat_map(|role| role.resources.iter())
            .collect()
    }
}

fn validate_address(address: &str, type_prefix: &str) -> CoreResult<()> {
    let valid = address.starts_with(type_prefix)
        && address.len() > type_prefix.len()
        && address.len() <= 256
        && !address.starts_with('-')
        && !address.chars().any(|c| c.is_control() || c.is_whitespace());
    if !valid {
        return Err(CoreError::new(
            ReasonCode::TemplatePlanFailed,
            "shaula_discover resource address is invalid or does not match declared terraform_type",
        ));
    }
    Ok(())
}

fn validate_import_id(import_id: &str) -> CoreResult<()> {
    if import_id.is_empty()
        || import_id.len() > MAX_IMPORT_ID_LEN
        || import_id.starts_with('-')
        || import_id.chars().any(char::is_control)
    {
        return Err(CoreError::new(
            ReasonCode::TemplatePlanFailed,
            "shaula_discover import_id is invalid",
        ));
    }
    Ok(())
}
