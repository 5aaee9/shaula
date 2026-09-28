//! A fenced replacement may reconstruct disposable files, never invent state.
use super::{digest_of, state_err, TemplateRuntime};
use shaula_core::ports::{TemplateDestroyRequest, TemplateOutcomeError};

impl TemplateRuntime {
    pub(super) async fn recover_workspace(
        &self,
        request: &TemplateDestroyRequest,
        input: &[u8],
    ) -> Result<(), TemplateOutcomeError> {
        let backend = self
            .http_backend
            .as_ref()
            .ok_or_else(|| state_err("recovery.backend"))?;
        backend
            .check_generation(&request.generation_id)
            .map_err(|_| state_err("recovery.identity"))?;
        if digest_of(input) != request.original_provenance.protected_input_digest {
            return Err(state_err("recovery.input"));
        }
        // Missing ordinary workspace is recoverable; unknown, legacy or
        // emergency state remains untouched and blocks automatic recovery.
        let workspace = &request.workspace_path;
        let empty = !workspace.exists()
            || std::fs::read_dir(workspace)
                .map_err(|_| state_err("recovery.workspace"))?
                .next()
                .is_none();
        if empty {
            self.prepare_create_flow(
                workspace,
                &request.artifact_dir,
                &request.pinned_artifact_digest,
                request.timeout,
            )
            .await?;
        } else {
            self.verify_http_workspace(workspace, false)?;
            let flow = self.open_flow(request.timeout).await?;
            let environment = backend
                .environment(&request.environment)
                .map_err(|_| state_err("recovery.environment"))?;
            self.initialize_workspace(&flow, workspace, &environment)
                .await?;
        }
        let value: serde_json::Value =
            serde_json::from_slice(input).map_err(|_| state_err("recovery.input"))?;
        let envelope = serde_json::from_value(value.get("shaula").cloned().unwrap_or_default())
            .map_err(|_| state_err("recovery.input"))?;
        let digest = super::input::write_protected_input(workspace, &envelope)
            .map_err(|_| state_err("recovery.input"))?;
        if digest != request.original_provenance.protected_input_digest {
            return Err(state_err("recovery.input"));
        }
        Ok(())
    }
}
