//! Shared exact-revision, schema-approved Template read projection.
use super::{unprocessable, ControlPlane};
use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    registry::{
        Actor, MutationError, Scope, TemplateProfileView, TemplateRevisionRow,
        TemplateSourceUpdate, TemplateSourceUpdateState,
    },
};

impl ControlPlane {
    pub(super) async fn template_read(
        &self,
        actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<TemplateProfileView, MutationError>> {
        if !actor.has(Scope::TemplateRead) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.read scope",
            )));
        }
        let Some(profile) = self.store.template_profile_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let revision = self
            .store
            .template_revision_get(key, profile.desired_revision)
            .await?;
        let bindings = match &revision {
            Some(row) => self.project_template_bindings(row).await?,
            None => None,
        };
        let source_update = match &revision {
            Some(row) => self.template_source_update(row).await?,
            None => None,
        };
        Ok(Ok(TemplateProfileView {
            source_update,
            key: key.to_string(),
            incarnation: profile.incarnation.clone(),
            desired_revision: profile.desired_revision,
            active_revision: profile.active_revision,
            runner_backend: self
                .active_template_backend(key, profile.active_revision)
                .await?,
            status: profile.status.clone(),
            bindings,
            validation_state: revision.as_ref().map(|r| r.state.clone()),
            validation_reason: revision.as_ref().and_then(|r| r.reason.clone()),
            references_in_use: self
                .store
                .template_references_in_use(key, self.now.now_unix_ms())
                .await?,
            platform: revision.as_ref().and_then(|r| r.platform.clone()),
            bindings_contract: revision.as_ref().and_then(|r| r.bindings_contract.clone()),
            bindings_present: revision
                .as_ref()
                .map(|r| r.bindings_present)
                .unwrap_or(false),
        }))
    }

    /// Compares the recorded source with the current catalog by exact content.
    async fn template_source_update(
        &self,
        row: &TemplateRevisionRow,
    ) -> CoreResult<Option<TemplateSourceUpdate>> {
        let Some(source_key) = row.source_key.clone() else {
            return Ok(None);
        };
        let source = self
            .store
            .template_source_get(&source_key)
            .await?
            // A Validating revision has no platform yet; admission already
            // matched its source's platform against the artifact manifest.
            .filter(|source| {
                row.platform
                    .as_deref()
                    .is_none_or(|platform| platform == source.platform)
            });
        let Some(source) = source else {
            return Ok(Some(TemplateSourceUpdate {
                source_key,
                state: TemplateSourceUpdateState::Unavailable,
                target_digest: None,
                target_engine: None,
            }));
        };
        let state = if source.artifact_digest == row.artifact_digest
            && source.engine_ref == row.engine_ref
        {
            TemplateSourceUpdateState::UpToDate
        } else {
            TemplateSourceUpdateState::Available
        };
        Ok(Some(TemplateSourceUpdate {
            source_key,
            state,
            target_digest: Some(source.artifact_digest),
            target_engine: Some(source.engine_ref),
        }))
    }

    pub(super) async fn project_template_bindings(
        &self,
        row: &TemplateRevisionRow,
    ) -> CoreResult<Option<serde_json::Value>> {
        let Some(text) = row.bindings_json.as_deref() else {
            return Ok(None);
        };
        let stored: serde_json::Map<String, serde_json::Value> = serde_json::from_str(text)
            .map_err(|_| {
                CoreError::new(
                    ReasonCode::StorageCorrupt,
                    "stored template bindings are invalid",
                )
            })?;
        // Missing or unreadable schema never grants access to binding values.
        let schema = self.bindings_schema_view(&row.artifact_digest).await;
        Ok(Some(serde_json::Value::Object(schema.project(&stored))))
    }
}
