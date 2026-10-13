//! Explicit VM-only exception for GitLab (spec 0043 §5): the single-registration
//! Runner token reaches protected tfvars, plan/state and cloud-init user-data
//! inside the same `config.toml` the container path copies. The boundary is
//! identical to the Forgejo VM exception; the Auth Profile token never enters.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{CoreError, CoreResult, ProfileManifest, ReasonCode, ShaulaInputEnvelope};
use crate::gitlab::GitlabBootstrapIdentity;
use crate::ports::gitlab::GitlabBootstrapMaterial;
use crate::secret::SecretString;

/// Opt-in delivery of one pre-registered GitLab Runner through VM cloud-init.
pub const GITLAB_VM_BOOTSTRAP_CONTRACT: &str = "shaula.gitlab-vm-cloud-init/v1";

/// Serializes only as `{"config_toml": ...}`, rendered by
/// [`GitlabBootstrapMaterial::config_toml`]; Debug never reveals the token.
#[derive(Clone, PartialEq)]
pub struct GitlabVmBootstrap {
    material: GitlabBootstrapMaterial,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    config_toml: String,
}

impl GitlabVmBootstrap {
    /// Copies the validated per-Runner registration, never the Auth Profile token.
    pub fn from_registration(material: &GitlabBootstrapMaterial) -> Self {
        Self {
            material: material.clone(),
        }
    }

    pub fn identity(&self) -> GitlabBootstrapIdentity {
        self.material.identity()
    }

    /// Only for redacting operation output that could echo the Runner token.
    pub fn token(&self) -> &str {
        self.material.token()
    }
}

impl std::fmt::Debug for GitlabVmBootstrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GitlabVmBootstrap([REDACTED])")
    }
}

impl Serialize for GitlabVmBootstrap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Wire {
            config_toml: self.material.config_toml(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GitlabVmBootstrap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let config = SecretString::new(Wire::deserialize(deserializer)?.config_toml);
        parse(config.expose())
            .map(|material| Self { material })
            .ok_or_else(|| serde::de::Error::custom("invalid GitLab VM bootstrap configuration"))
    }
}

/// Accepts exactly the canonical `shell` rendering; anything else (another
/// executor, extra keys, a non-`glrt-` token) is refused rather than relayed.
fn parse(config: &str) -> Option<GitlabBootstrapMaterial> {
    if config.len() > 8192 {
        return None;
    }
    let body = config.strip_prefix("concurrent = 1\ncheck_interval = 3\n\n[[runners]]\n")?;
    let mut lines = body.lines();
    let mut quoted = |key: &str| {
        lines
            .next()?
            .strip_prefix(&format!("  {key} = \""))?
            .strip_suffix('"')
            .map(str::to_owned)
    };
    let name = quoted("name")?;
    let url = quoted("url")?;
    let id = lines.next()?.strip_prefix("  id = ")?.parse().ok()?;
    let token = SecretString::new(
        lines
            .next()?
            .strip_prefix("  token = \"")?
            .strip_suffix('"')?,
    );
    let material = GitlabBootstrapMaterial::new(url, id, name, token).ok()?;
    (material.config_toml() == config).then_some(material)
}

impl ProfileManifest {
    pub(super) fn validate_gitlab_vm_bootstrap(&self) -> CoreResult<()> {
        let Some(contract) = self.gitlab_vm_bootstrap_contract.as_deref() else {
            return Ok(());
        };
        if contract != GITLAB_VM_BOOTSTRAP_CONTRACT
            || !matches!(
                self.platform.as_str(),
                "proxmox" | "aws" | "tencentcloud" | "alicloud"
            )
            || self.vm_image_contract.is_none()
            || self.container_bootstrap_contract.is_some()
            || self.input_contract_version != 1
            || !self.supports_runner_backend("gitlab")
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "unsupported GitLab VM cloud-init bootstrap contract",
            ));
        }
        Ok(())
    }

    /// GitLab Create admission: VM artifacts through their cloud-init
    /// contract (`shell` executor only), everything else through
    /// [`Self::validate_gitlab_target`].
    pub fn validate_gitlab_generation_target(&self) -> CoreResult<()> {
        if self.gitlab_vm_bootstrap_contract.is_none() {
            return self.validate_gitlab_target();
        }
        self.validate_for_provider(crate::fleet::FleetProviderKind::Gitlab)?;
        self.validate_gitlab_vm_bootstrap()
    }
}

impl ShaulaInputEnvelope {
    /// The VM token must belong to exactly the GitLab registration named by
    /// the non-secret identity, on an envelope without any other credential.
    pub(super) fn validate_gitlab_vm(&self) -> CoreResult<()> {
        let Some(vm) = &self.gitlab_vm else {
            return Ok(());
        };
        if self.gitlab.as_ref() != Some(&vm.identity())
            || self.forgejo.is_some()
            || self.forgejo_vm.is_some()
            || !self.jit_config.is_empty()
            || self.generation.scale_set_id.is_some()
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "VM bootstrap credentials require an exclusive GitLab identity",
            ));
        }
        Ok(())
    }
}
