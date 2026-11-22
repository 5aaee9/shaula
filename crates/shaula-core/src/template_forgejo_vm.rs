//! Explicit VM-only exception: ephemeral Runner credentials in protected cloud-init input.
//! Cloud metadata, NoCloud seeds, tfvars/plans/state remain credential-grade.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{CoreResult, ProfileManifest};
use crate::{ports::forgejo::ForgejoBootstrapMaterial, secret::SecretString};

/// Opt-in delivery of one ephemeral Runner token through VM cloud-init.
pub const FORGEJO_VM_BOOTSTRAP_CONTRACT: &str = "shaula.forgejo-vm-cloud-init/v1";

/// Serializable only for protected VM input; Debug never reveals the token.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgejoVmBootstrap {
    #[serde(
        serialize_with = "serialize_token",
        deserialize_with = "deserialize_token"
    )]
    token: SecretString,
}

impl ForgejoVmBootstrap {
    /// Copies a validated per-Runner token, never the Forgejo management credential.
    pub fn from_registration(material: &ForgejoBootstrapMaterial) -> Self {
        Self {
            token: SecretString::new(material.token()),
        }
    }
}

impl std::fmt::Debug for ForgejoVmBootstrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ForgejoVmBootstrap([REDACTED])")
    }
}

fn serialize_token<S: Serializer>(token: &SecretString, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(token.expose())
}

fn deserialize_token<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SecretString, D::Error> {
    let token = SecretString::new(String::deserialize(deserializer)?);
    if token.expose().is_empty()
        || token.expose().len() > 4096
        || token.expose().chars().any(char::is_control)
    {
        return Err(serde::de::Error::custom(
            "invalid Forgejo VM bootstrap credential",
        ));
    }
    Ok(token)
}

#[cfg(test)]
#[path = "template_forgejo_vm_tests.rs"]
mod tests;

impl ProfileManifest {
    pub(super) fn validate_forgejo_vm_bootstrap(&self) -> CoreResult<()> {
        self.validate_vm_bootstrap_contract(
            self.forgejo_vm_bootstrap_contract.as_deref(),
            FORGEJO_VM_BOOTSTRAP_CONTRACT,
            "forgejo",
            "unsupported Forgejo VM cloud-init bootstrap contract",
        )
    }
}
