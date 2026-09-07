//! Profile manifest parsing: the artifact's `profile.yaml` is the sole
//! authority for `platform` and `bindings_contract`.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::template::ProfileManifest;

/// Parses and structurally validates the manifest of an admitted artifact.
pub fn parse_manifest(yaml: &str) -> CoreResult<ProfileManifest> {
    let manifest: ProfileManifest = serde_yaml::from_str(yaml).map_err(|e| {
        CoreError::new(
            ReasonCode::TemplateInvalid,
            format!("manifest invalid: {e}"),
        )
    })?;
    manifest.validate()?;
    Ok(manifest)
}

/// Declared artifact shape every profile must carry.
pub const REQUIRED_ARTIFACT_FILES: &[&str] = &[
    "profile.yaml",
    ".terraform.lock.hcl",
    "schemas/bindings.schema.json",
    "schemas/parameters.schema.json",
];

/// Verifies required files exist in the extracted artifact directory.
pub fn verify_artifact_shape(dir: &std::path::Path) -> CoreResult<()> {
    for required in REQUIRED_ARTIFACT_FILES {
        let path = dir.join(required);
        if !path.is_file() {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                format!("artifact missing required file {required}"),
            ));
        }
    }
    // Reject undeclared top-level executables or nested Terraform modules.
    for entry in std::fs::read_dir(dir).map_err(|e| {
        CoreError::new(
            ReasonCode::TemplateInvalid,
            format!("artifact unreadable: {e}"),
        )
    })? {
        let entry = entry.map_err(|e| {
            CoreError::new(
                ReasonCode::TemplateInvalid,
                format!("artifact unreadable: {e}"),
            )
        })?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".exe")
            || name.ends_with(".sh")
            || name.ends_with(".cmd")
            || name.ends_with(".bat")
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "executable entry inside artifact",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &str = r#"
api_version: shaula.io/template-profile/v1
kind: RunnerTemplateProfile
platform: kubernetes
runtime:
  protocol: terraform-cli/v1
  engine: terraform
  root_module: .
  required_version: ">= 1.9, < 2.0"
bindings_contract: shaula.bindings.kubernetes/v1
schemas:
  bindings: schemas/bindings.schema.json
  parameters: schemas/parameters.schema.json
managed_resource_shape:
  - role: bootstrap
    terraform_type: kubernetes_secret_v1
    exact_count: 1
  - role: runner
    terraform_type: kubernetes_pod_v1
    exact_count: 1
runner_image_digests:
  - ghcr.io/actions/actions-runner:2.323.0@sha256:3f2a1b9c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8
runtime_policy_digest: sha256:policy-v1
"#;

    #[test]
    fn parses_valid_manifest() {
        let manifest = parse_manifest(VALID_MANIFEST).unwrap();
        assert_eq!(
            manifest.platform(),
            shaula_core::template::TemplatePlatform::Kubernetes
        );
        assert_eq!(manifest.bindings_contract, "shaula.bindings.kubernetes/v1");
    }

    #[test]
    fn rejects_invalid_manifest() {
        assert!(parse_manifest("api_version: wrong\n").is_err());
        assert!(parse_manifest("not: [valid").is_err());
    }

    #[test]
    fn artifact_shape_requires_lock_and_schemas() {
        let dir = tempfile::tempdir().unwrap();
        assert!(verify_artifact_shape(dir.path()).is_err());
        std::fs::create_dir(dir.path().join("schemas")).unwrap();
        for file in REQUIRED_ARTIFACT_FILES {
            std::fs::write(dir.path().join(file), "x").unwrap();
        }
        assert!(verify_artifact_shape(dir.path()).is_ok());
        std::fs::write(dir.path().join("evil.cmd"), "rem").unwrap();
        assert!(
            verify_artifact_shape(dir.path()).is_err(),
            "executables rejected"
        );
    }
}
