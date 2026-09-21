//! Load the frozen Template Revision and reject a backend mismatch before JIT.

use serde_json::{Map, Value};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetProviderKind;
use shaula_core::registry::GenerationRecord;
use shaula_core::template::ProfileManifest;

use super::FleetSupervisor;

impl FleetSupervisor {
    pub(super) async fn template_material(
        &self,
        artifact_dir: &std::path::Path,
        record: &GenerationRecord,
        parameters: &Map<String, Value>,
    ) -> CoreResult<(ProfileManifest, Map<String, Value>, String)> {
        let body = std::fs::read_to_string(artifact_dir.join("profile.yaml")).map_err(|_| {
            CoreError::new(ReasonCode::TemplateInvalid, "admitted manifest unreadable")
        })?;
        let manifest: ProfileManifest = serde_yaml::from_str(&body).map_err(|_| {
            CoreError::new(ReasonCode::TemplateInvalid, "admitted manifest invalid")
        })?;
        let (json, digest) = self
            .handoff
            .template_protected_bindings(&record.template_profile_key, record.template_revision)
            .await?
            .ok_or_else(|| {
                CoreError::new(ReasonCode::TemplateInvalid, "admitted bindings missing")
            })?;
        let bindings = serde_json::from_str(&json).map_err(|_| {
            CoreError::new(ReasonCode::TemplateInvalid, "admitted bindings invalid")
        })?;
        manifest.validate_bindings_for_provider(&bindings, FleetProviderKind::Github)?;
        if manifest.container_bootstrap_contract.is_some() {
            manifest.selected_runner_image(&bindings, parameters)?;
        }
        Ok((manifest, bindings, digest))
    }
}
