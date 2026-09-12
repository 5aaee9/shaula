//! Official-container bootstrap opt-in; retained manifests keep their original contract.
use super::{CoreError, CoreResult, ProfileManifest, ReasonCode};

pub const CONTAINER_BOOTSTRAP_CONTRACT: &str = "shaula.container-bootstrap/v1";

impl ProfileManifest {
    /// The first Forgejo container contract supports host execution inside the
    /// runner container, not access to an external Docker daemon or VM bootstrap.
    pub fn validate_forgejo_targets(&self, labels: &[String]) -> CoreResult<()> {
        self.validate_for_provider(crate::fleet::FleetProviderKind::Forgejo)?;
        crate::forgejo::validate_labels(labels)?;
        if !matches!(self.platform.as_str(), "docker" | "kubernetes")
            || self.container_bootstrap_contract.as_deref() != Some(CONTAINER_BOOTSTRAP_CONTRACT)
            || labels.iter().any(|label| !label.ends_with(":host"))
        {
            return Err(CoreError::new(ReasonCode::TemplateInvalid,
                "Forgejo container bootstrap requires explicit :host labels; Docker-in-Docker and VM targets are not admitted"));
        }
        Ok(())
    }

    /// Admission for new publications and Create, never retained reads or Destroy.
    pub fn validate_new_container_profile(&self) -> CoreResult<()> {
        self.validate()?;
        if matches!(self.platform.as_str(), "docker" | "kubernetes")
            && self.container_bootstrap_contract.is_none()
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "new container runners require official images and host-owned bootstrap",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_container_bootstrap(&self) -> CoreResult<()> {
        let Some(contract) = &self.container_bootstrap_contract else {
            return Ok(());
        };
        let binding = match self.platform.as_str() {
            "docker" => "shaula.bindings.docker/v1",
            "kubernetes" => "shaula.bindings.kubernetes/v1",
            _ => "",
        };
        if contract != CONTAINER_BOOTSTRAP_CONTRACT
            || binding.is_empty()
            || self.bindings_contract != binding
            || self.input_contract_version != 1
            || self.setup_info_contract.is_some()
            || self
                .runner_image_digests
                .iter()
                .any(|image| !official_runner_image(&self.runner_backend, image))
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "unsupported container bootstrap contract or non-official runner image",
            ));
        }
        Ok(())
    }
}

fn official_runner_image(backend: &str, image: &str) -> bool {
    match backend {
        "github" => {
            image.starts_with("ghcr.io/actions/actions-runner:")
                || image.starts_with("ghcr.io/actions/actions-runner@sha256:")
        }
        // The general image contract checks the digest. A versionless digest
        // does not establish one-job support until an exact R4 tuple is admitted.
        "forgejo" => image
            .strip_prefix("code.forgejo.org/forgejo/runner:")
            .and_then(|reference| reference.split_once("@sha256:"))
            .is_some_and(|(version, _)| crate::forgejo::supports_runner_version(version)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn official_bootstrap_rejects_custom_images_and_legacy_delivery_mix() -> TestResult {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates/docker/profile.yaml");
        let mut manifest: ProfileManifest = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
        manifest.container_bootstrap_contract = Some(CONTAINER_BOOTSTRAP_CONTRACT.into());
        manifest.runner_image_digests = vec![format!(
            "ghcr.io/actions/actions-runner:2.337.0@sha256:{}",
            "a".repeat(64)
        )];
        manifest.validate()?;
        let original = manifest.clone();
        manifest.runner_image_digests[0] = manifest.runner_image_digests[0].replace(
            "ghcr.io/actions/actions-runner",
            "registry.test/custom-runner",
        );
        assert!(manifest.validate().is_err());
        manifest = original.clone();
        manifest.input_contract_version = 2;
        manifest.setup_info_contract = Some(super::super::SETUP_INFO_CONTRACT.into());
        assert!(manifest.validate().is_err());
        manifest = original;
        manifest.platform = "other".into();
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn forgejo_container_requires_provable_runner_version() -> TestResult {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates/docker/profile.yaml");
        let mut manifest: ProfileManifest = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
        manifest.runner_backend = "forgejo".into();
        for tag in [":12.7.0", ":13.0.0-rc1", ":latest", ""] {
            manifest.runner_image_digests = vec![format!(
                "code.forgejo.org/forgejo/runner{tag}@sha256:{}",
                "a".repeat(64)
            )];
            assert!(manifest.validate_new_container_profile().is_err());
        }
        manifest.runner_image_digests = vec![format!(
            "code.forgejo.org/forgejo/runner:13.1.0@sha256:{}",
            "a".repeat(64)
        )];
        manifest.validate_new_container_profile()?;
        Ok(())
    }

    #[test]
    fn legacy_container_manifest_is_readable_but_cannot_authorize_new_runners() -> TestResult {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates/docker/profile.yaml");
        let mut manifest: ProfileManifest = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
        manifest.validate_new_container_profile()?;
        manifest.container_bootstrap_contract = None;
        manifest.runner_image_digests = vec![format!(
            "registry.test/custom:old@sha256:{}",
            "a".repeat(64)
        )];
        manifest.validate()?;
        assert!(manifest.validate_new_container_profile().is_err());
        manifest.platform = "virtual-machine".into();
        manifest.validate_new_container_profile()?;
        Ok(())
    }
}
