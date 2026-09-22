//! Check the backend frozen in a Template Revision before admitting or following it.

use serde_json::{Map, Value};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::{FleetProviderKind, FleetSpec};
use shaula_core::template::ProfileManifest;
use shaula_core::template_pool::ResolvedTemplatePoolMember;

use super::ControlPlane;

impl ControlPlane {
    pub(super) async fn active_template_backend(
        &self,
        key: &str,
        revision: Option<i64>,
    ) -> CoreResult<Option<FleetProviderKind>> {
        let Some(revision) = revision else {
            return Ok(None);
        };
        let Some(row) = self.store.template_revision_get(key, revision).await? else {
            return Ok(None);
        };
        let Some(yaml) = self.store.artifact_manifest(&row.artifact_digest).await? else {
            return Ok(None);
        };
        let json = row.bindings_json.as_deref().unwrap_or("{}");
        // Read-only discovery fails closed, without projecting protected bindings.
        let backend = serde_yaml::from_str::<ProfileManifest>(&yaml)
            .ok()
            .and_then(|manifest| {
                let bindings = serde_json::from_str(json).ok()?;
                manifest.validate().ok()?;
                match manifest.runner_backend_for_bindings(&bindings).ok()? {
                    "github" => Some(FleetProviderKind::Github),
                    "forgejo" => Some(FleetProviderKind::Forgejo),
                    _ => None,
                }
            });
        Ok(backend)
    }

    pub(super) async fn validate_template_backend(
        &self,
        profile: &str,
        revision: i64,
        artifact: &str,
        spec: &FleetSpec,
        parameters: &Map<String, Value>,
    ) -> CoreResult<()> {
        let yaml = self
            .store
            .artifact_manifest(artifact)
            .await?
            .ok_or_else(|| {
                CoreError::new(ReasonCode::TemplateInvalid, "template manifest is missing")
            })?;
        let manifest: ProfileManifest = serde_yaml::from_str(&yaml).map_err(|_| {
            CoreError::new(ReasonCode::TemplateInvalid, "template manifest is invalid")
        })?;
        let (json, _) = self
            .store
            .template_protected_bindings(profile, revision)
            .await?
            .ok_or_else(|| {
                CoreError::new(ReasonCode::TemplateInvalid, "template bindings are missing")
            })?;
        let bindings = serde_json::from_str(&json).map_err(|_| {
            CoreError::new(ReasonCode::TemplateInvalid, "template bindings are invalid")
        })?;
        manifest.validate_bindings_for_provider(&bindings, spec.kind)?;
        if manifest.container_bootstrap_contract.is_some() {
            manifest.selected_runner_image(&bindings, parameters)?;
        }
        if spec.kind == FleetProviderKind::Forgejo {
            let labels = spec
                .forgejo
                .as_ref()
                .map(|section| section.labels.as_slice())
                .unwrap_or_default();
            manifest.validate_forgejo_targets(labels)?;
        }
        Ok(())
    }

    pub(super) async fn validate_pool_backends(
        &self,
        members: &[ResolvedTemplatePoolMember],
        spec: &FleetSpec,
    ) -> CoreResult<()> {
        for member in members {
            self.validate_template_backend(
                &member.template_profile_key,
                member.template_revision,
                &member.template_artifact_digest,
                spec,
                &member.template_inputs,
            )
            .await?;
        }
        Ok(())
    }
}
