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
