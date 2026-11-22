//! A fenced replacement may reconstruct disposable files, never invent state.
use super::{digest_of, state_err, TemplateRuntime};
use shaula_core::ports::{TemplateDestroyRequest, TemplateOutcomeError, TemplateReconcileRequest};
use std::path::Path;
use std::time::Duration;

struct RecoveryTarget<'a> {
    generation_id: &'a str,
    workspace: &'a Path,
    artifact_dir: &'a Path,
    pinned_artifact_digest: &'a str,
    environment: &'a [(String, String)],
    timeout: Duration,
    expected_input_digest: &'a str,
}

impl TemplateRuntime {
    pub(super) async fn recover_workspace(
        &self,
        request: &TemplateDestroyRequest,
        input: &[u8],
    ) -> Result<(), TemplateOutcomeError> {
        self.recover_workspace_target(
            RecoveryTarget {
                generation_id: &request.generation_id,
                workspace: &request.workspace_path,
                artifact_dir: &request.artifact_dir,
                pinned_artifact_digest: &request.pinned_artifact_digest,
                environment: &request.environment,
                timeout: request.timeout,
                expected_input_digest: &request.original_provenance.protected_input_digest,
            },
            input,
        )
        .await
    }

    pub(super) async fn recover_reconcile_workspace(
        &self,
        request: &TemplateReconcileRequest,
        input: &[u8],
    ) -> Result<(), TemplateOutcomeError> {
        self.recover_workspace_target(
            RecoveryTarget {
                generation_id: &request.generation_id,
                workspace: &request.workspace_path,
                artifact_dir: &request.artifact_dir,
                pinned_artifact_digest: &request.pinned_artifact_digest,
                environment: &request.environment,
                timeout: request.timeout,
                expected_input_digest: &request.original_provenance.protected_input_digest,
            },
            input,
        )
        .await
    }

    async fn recover_workspace_target(
        &self,
        target: RecoveryTarget<'_>,
        input: &[u8],
    ) -> Result<(), TemplateOutcomeError> {
        let backend = self
            .http_backend
            .as_ref()
            .ok_or_else(|| state_err("recovery.backend"))?;
        backend
            .check_generation(target.generation_id)
            .map_err(|_| state_err("recovery.identity"))?;
        if digest_of(input) != target.expected_input_digest {
            return Err(state_err("recovery.input"));
        }
        // Missing ordinary workspace is recoverable; unknown, legacy or
        // emergency state remains untouched and blocks automatic recovery.
        let empty = !target.workspace.exists()
            || std::fs::read_dir(target.workspace)
                .map_err(|_| state_err("recovery.workspace"))?
                .next()
                .is_none();
        if empty {
            self.prepare_create_flow(
                target.workspace,
                target.artifact_dir,
                target.pinned_artifact_digest,
                target.timeout,
            )
            .await?;
        } else {
            self.verify_http_workspace(target.workspace, false)?;
            let flow = self.open_flow(target.timeout).await?;
            let environment = backend
                .environment(target.environment)
                .map_err(|_| state_err("recovery.environment"))?;
            self.initialize_workspace(&flow, target.workspace, &environment)
                .await?;
        }
        let value: serde_json::Value =
            serde_json::from_slice(input).map_err(|_| state_err("recovery.input"))?;
        let envelope = serde_json::from_value(value.get("shaula").cloned().unwrap_or_default())
            .map_err(|_| state_err("recovery.input"))?;
        let digest = super::input::write_protected_input(target.workspace, &envelope)
            .map_err(|_| state_err("recovery.input"))?;
        if digest != target.expected_input_digest {
            return Err(state_err("recovery.input"));
        }
        Ok(())
    }
}
