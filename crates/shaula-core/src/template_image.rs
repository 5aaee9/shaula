//! Image authority: immutable pins or an explicit operator-managed VM contract.

use super::{CoreError, CoreResult, ProfileManifest, ReasonCode};

pub const PROXMOX_VM_IMAGE_CONTRACT: &str = "shaula.proxmox-template/v1";

impl ProfileManifest {
    pub(super) fn validate_image_contract(&self) -> CoreResult<()> {
        if let Some(contract) = &self.vm_image_contract {
            if contract != PROXMOX_VM_IMAGE_CONTRACT
                || self.platform != "proxmox"
                || self.bindings_contract != "shaula.bindings.proxmox/v1"
                || self.input_contract_version != 1
                || self.setup_info_contract.is_some()
                || self.container_bootstrap_contract.is_some()
                || !self.runner_image_digests.is_empty()
            {
                return Err(CoreError::new(
                    ReasonCode::TemplateInvalid,
                    "unsupported VM image contract or conflicting image/bootstrap declaration",
                ));
            }
            return Ok(());
        }

        if self.runner_image_digests.is_empty() || self.runner_image_digests.len() > 8 {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "runner_image_digests must declare 1..=8 admitted images",
            ));
        }
        for image in &self.runner_image_digests {
            // A mutable tag cannot establish the conformance image subject.
            let Some((_, digest)) = image.trim().split_once("@sha256:") else {
                return Err(CoreError::new(
                    ReasonCode::TemplateInvalid,
                    "runner_image_digests entries must be content pins (repo@sha256:<64 hex>)",
                ));
            };
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(CoreError::new(
                    ReasonCode::TemplateInvalid,
                    "runner_image_digests sha256 digest must be exactly 64 hex characters",
                ));
            }
        }
        let mut images: Vec<&str> = self
            .runner_image_digests
            .iter()
            .map(|image| image.as_str())
            .collect();
        images.sort_unstable();
        images.dedup();
        if images.len() != self.runner_image_digests.len() {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "duplicate runner_image_digests entries",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "template_image_tests.rs"]
mod tests;
