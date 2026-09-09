//! A recorded source must identify the exact reviewed artifact and engine.

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::registry::{MutationError, TemplateProfilePut};
use shaula_core::template::ProfileManifest;

use super::{unprocessable, ControlPlane};

impl ControlPlane {
    pub(super) async fn validate_template_source(
        &self,
        payload: &TemplateProfilePut,
        manifest: &ProfileManifest,
        update_base_source: Option<&str>,
    ) -> CoreResult<Result<(), MutationError>> {
        let Some(source_key) = &payload.source_key else {
            return Ok(Ok(()));
        };
        if update_base_source.is_some_and(|base| base != source_key) {
            return Ok(Err(unprocessable(
                ReasonCode::TemplateInvalid,
                "updating from a different source requires a new revision publication",
            )));
        }
        let Some(source) = self.store.template_source_get(source_key).await? else {
            return Ok(Err(unprocessable(
                ReasonCode::TemplateInvalid,
                "template source is unavailable",
            )));
        };
        if source.artifact_digest != payload.artifact_digest
            || source.engine_ref != payload.engine_ref
            || source.platform != manifest.platform
        {
            return Ok(Err(unprocessable(
                ReasonCode::TemplateInvalid,
                "template source does not match the reviewed artifact, engine and platform",
            )));
        }
        Ok(Ok(()))
    }
}
