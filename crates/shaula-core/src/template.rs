//! Template Profile domain: profile keys, manifest contracts and the fixed
//! `shaula` / `shaula_result` envelopes (spec 0004 section 3).

use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::{CoreError, CoreResult, ReasonCode};

/// Stable logical key of a Template Profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TemplateProfileKey(String);

impl TemplateProfileKey {
    /// Strict parse (R8-01): the shared stable-identifier predicate, no
    /// normalization — the validated bytes are exactly the stored bytes.
    pub fn new(value: impl Into<String>) -> CoreResult<Self> {
        let value = value.into();
        if !crate::auth::is_stable_identifier(&value) {
            return Err(CoreError::new(
                ReasonCode::SpecInvalid,
                "template profile key is not a stable identifier",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Platform identity derived ONLY from the admitted artifact manifest.
/// HTTP bodies, bindings and attestations can never override it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TemplatePlatform {
    Kubernetes,
    Docker,
    Proxmox,
    Other,
}

impl TemplatePlatform {
    /// Bounded metric label; unrecognized values map to `other`.
    pub fn from_manifest_value(value: &str) -> Self {
        match value {
            "kubernetes" => TemplatePlatform::Kubernetes,
            "docker" => TemplatePlatform::Docker,
            "proxmox" => TemplatePlatform::Proxmox,
            _ => TemplatePlatform::Other,
        }
    }

    pub fn metric_label(self) -> &'static str {
        match self {
            TemplatePlatform::Kubernetes => "kubernetes",
            TemplatePlatform::Docker => "docker",
            TemplatePlatform::Proxmox => "proxmox",
            TemplatePlatform::Other => "other",
        }
    }
}

/// Managed resource shape role declared by the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedResourceRole {
    pub role: String,
    pub terraform_type: String,
    pub exact_count: u64,
}

/// The versioned `shaula-profile` manifest inside an admitted artifact.
/// The manifest declares protocol and plan shape but cannot define custom
/// commands or hooks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileManifest {
    pub api_version: String,
    pub kind: String,
    pub platform: String,
    pub runtime: ManifestRuntime,
    pub bindings_contract: String,
    pub schemas: ManifestSchemas,
    pub managed_resource_shape: Vec<ManagedResourceRole>,
    /// The exact runner image set the publisher admits and the conformance
    /// run attested. The registry compares the attestation subject against
    /// THIS list — the only authority (spec 0005 §5.1). It is empty only
    /// when `vm_image_contract` explicitly admits an operator-managed VM image.
    pub runner_image_digests: Vec<String>,
    /// Publisher-declared digest of the pinned runtime policy document.
    pub runtime_policy_digest: String,
    #[serde(
        default = "default_input_contract_version",
        skip_serializing_if = "is_v1_input"
    )]
    pub input_contract_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_info_contract: Option<String>,
    /// Explicit permission for the Runtime's fixed post-apply container bootstrap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_bootstrap_contract: Option<String>,
    /// Explicit trust in an operator-managed Proxmox base VM; this does not
    /// claim an immutable image digest or authorize a Runtime bootstrap hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm_image_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRuntime {
    pub protocol: String,
    pub engine: String,
    pub root_module: String,
    pub required_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSchemas {
    pub bindings: String,
    pub parameters: String,
}

pub const MANIFEST_API_VERSION: &str = "shaula.io/template-profile/v1";
pub const MANIFEST_KIND: &str = "RunnerTemplateProfile";
pub const SUPPORTED_PROTOCOL: &str = "terraform-cli/v1";
pub const SUPPORTED_ENGINE: &str = "terraform";

/// The accepted suite for conformance evidence. Subject and envelope must
/// both name it to verify; conformance does not control activation (spec 0017).
pub const ACCEPTED_CONFORMANCE_SUITE: (&str, &str) = ("shaula-template-conformance", "v1");

impl ProfileManifest {
    /// Static structural validation; platform identity stays an opaque
    /// string mapped through [`TemplatePlatform::from_manifest_value`].
    pub fn validate(&self) -> CoreResult<()> {
        self.validate_container_bootstrap()?;
        match (
            self.input_contract_version,
            self.setup_info_contract.as_deref(),
        ) {
            (1, None) | (2, Some(SETUP_INFO_CONTRACT)) => {}
            _ => {
                return Err(CoreError::new(
                    ReasonCode::TemplateInvalid,
                    "unsupported input/setup-info contract combination",
                ))
            }
        }
        if self.api_version != MANIFEST_API_VERSION {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "unsupported manifest api_version",
            ));
        }
        if self.kind != MANIFEST_KIND {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "unsupported manifest kind",
            ));
        }
        if self.runtime.protocol != SUPPORTED_PROTOCOL {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "unsupported runtime protocol",
            ));
        }
        if self.runtime.engine != SUPPORTED_ENGINE {
            // OpenTofu is gated by compatibility; fail closed here.
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "unsupported engine; v1 is Terraform-only",
            ));
        }
        if self.runtime.root_module.trim().is_empty() {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "root_module must be set",
            ));
        }
        if !required_version_supported(&self.runtime.required_version) {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "required_version uses an unsupported constraint grammar",
            ));
        }
        if self.bindings_contract.is_empty() || self.bindings_contract.len() > 128 {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "bindings_contract must be 1..=128 characters",
            ));
        }
        if self.managed_resource_shape.is_empty() || self.managed_resource_shape.len() > 8 {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "managed_resource_shape must declare 1..=8 roles",
            ));
        }
        self.validate_image_contract()?;
        if self.runtime_policy_digest.trim().is_empty() {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "runtime_policy_digest must be set",
            ));
        }
        let mut roles: Vec<&str> = self
            .managed_resource_shape
            .iter()
            .map(|r| r.role.as_str())
            .collect();
        roles.sort_unstable();
        let before = roles.len();
        roles.dedup();
        if roles.len() != before {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "duplicate managed resource roles",
            ));
        }
        for shape in &self.managed_resource_shape {
            if shape.role.is_empty() || shape.terraform_type.is_empty() || shape.exact_count == 0 {
                return Err(CoreError::new(
                    ReasonCode::TemplateInvalid,
                    "invalid managed resource shape entry",
                ));
            }
        }
        Ok(())
    }

    pub fn platform(&self) -> TemplatePlatform {
        TemplatePlatform::from_manifest_value(&self.platform)
    }
}

/// The opaque, server-issued commitment binding envelopes to an exact
/// immutable binding revision. Wire name `bindings_digest`. It is never an
/// unkeyed digest of sensitive plaintext and cannot validate secret guesses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BindingsDigest(pub String);

impl BindingsDigest {
    /// Issued server-side with a real HMAC over the revision identity;
    /// a keyed construction prevents offline validation of secret guesses.
    ///
    /// # Errors
    /// Only when the HMAC implementation rejects the server key material
    /// — an operational configuration fault, surfaced instead of
    /// panicked (strict `-D warnings` policy).
    pub fn from_keyed_material(revision_id: &str, server_key: &[u8]) -> CoreResult<Self> {
        use hmac::Mac;
        let mut mac = hmac::Hmac::<Sha256>::new_from_slice(server_key).map_err(|e| {
            CoreError::new(
                ReasonCode::Internal,
                format!("bindings digest server key rejected: {e}"),
            )
        })?;
        mac.update(b"shaula.bindings_digest.v1");
        mac.update(revision_id.as_bytes());
        let out = mac.finalize().into_bytes();
        Ok(Self(format!("bd1_{}", hex::encode(out))))
    }
}

#[path = "template_envelope.rs"]
pub mod envelope;
pub use envelope::{
    AttestationInsert, GenerationIdentity, ResultResource, ShaulaInputEnvelope,
    ShaulaResultEnvelope, INPUT_CONTRACT_VERSION, MAX_RESULT_RESOURCES, RESULT_CONTRACT_VERSION,
};

#[path = "template_setup_info.rs"]
mod setup_info;
pub use setup_info::{SetupInfoDescriptor, SETUP_INFO_CONTRACT};

#[path = "template_container.rs"]
mod container;
pub use container::CONTAINER_BOOTSTRAP_CONTRACT;

#[path = "template_image.rs"]
mod image;
pub use image::PROXMOX_VM_IMAGE_CONTRACT;

fn default_input_contract_version() -> u32 {
    1
}
fn is_v1_input(version: &u32) -> bool {
    *version == 1
}

#[cfg(test)]
#[path = "template_tests.rs"]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

#[path = "template_insert.rs"]
pub mod insert;
pub use insert::TemplateRevisionInsert;

#[path = "template_version.rs"]
pub mod version;
pub(crate) use version::required_version_supported;
pub use version::version_satisfies;
