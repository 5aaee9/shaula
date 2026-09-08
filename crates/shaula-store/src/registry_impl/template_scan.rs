//! Reads artifact authority outside the automatic-activation transaction.

use sha2::{Digest, Sha256};
use shaula_core::error::CoreResult;
use shaula_core::registry::ControlPlaneStore;

use crate::template_activation::{ValidatedTemplate, ValidationCommit};

use super::{core_err, ScanReport, SqliteControlPlane};

impl SqliteControlPlane {
    pub(super) async fn scan_templates(&self, now: i64, report: &mut ScanReport) -> CoreResult<()> {
        for profile in self
            .store
            .template_profiles_list()
            .await
            .map_err(core_err)?
        {
            if profile.deletion_requested
                || matches!(profile.status.as_str(), "Retiring" | "Retired")
                || profile.active_revision == Some(profile.desired_revision)
            {
                continue;
            }
            let Some(candidate) = self
                .store
                .template_revision_get(&profile.key, profile.desired_revision)
                .await
                .map_err(core_err)?
            else {
                continue;
            };
            if !matches!(candidate.state.as_str(), "Validating" | "Ready") {
                continue;
            }
            let validation = self
                .validate_template_artifact(&candidate.artifact_digest)
                .await?;
            match self
                .store
                .template_validation_commit(&profile, &candidate, validation, now)
                .await
                .map_err(core_err)?
            {
                ValidationCommit::Activated => {
                    report.candidates_ready += 1;
                    report.candidates_activated += 1;
                }
                ValidationCommit::Rejected => report.candidates_rejected += 1,
                ValidationCommit::Superseded => {}
            }
        }
        Ok(())
    }

    async fn validate_template_artifact(
        &self,
        digest: &str,
    ) -> CoreResult<Result<ValidatedTemplate, &'static str>> {
        let Some(manifest_json) = self.artifact_manifest(digest).await? else {
            return Ok(Err("ArtifactNotPublished"));
        };
        if !self.artifact_shape_ok(digest).await? {
            return Ok(Err("ArtifactShapeInvalid"));
        }
        let Ok(manifest) =
            serde_yaml::from_str::<shaula_core::template::ProfileManifest>(&manifest_json)
        else {
            return Ok(Err("ManifestInvalid"));
        };
        if manifest.validate().is_err() {
            return Ok(Err("ManifestInvalid"));
        }
        let Some(lock) = self.artifact_lock_file(digest).await? else {
            return Ok(Err("ArtifactShapeInvalid"));
        };
        if shaula_core::lockfile::parse_lock_providers(&lock).is_err() {
            return Ok(Err("DependencyLockInvalid"));
        }
        Ok(Ok(ValidatedTemplate {
            platform: manifest.platform,
            bindings_contract: manifest.bindings_contract,
            manifest_json,
            lock_digest: format!("sha256:{}", hex::encode(Sha256::digest(lock.as_bytes()))),
        }))
    }
}
