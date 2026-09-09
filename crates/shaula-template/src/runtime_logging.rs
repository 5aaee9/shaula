use super::TemplateRuntime;
use crate::{operation_capture::InvocationGuard, operation_sanitize::SensitiveValues};
use shaula_core::{
    operation_log::OperationLogSink,
    ports::{
        DestroyClassification, TemplateCreateRequest, TemplateCreateResult, TemplateDestroyRequest,
        TemplateOutcomeError,
    },
};
use std::{path::Path, sync::Arc, time::Duration};

impl TemplateRuntime {
    pub(super) fn validate_input_contract(
        &self,
        artifact: &Path,
        input: &shaula_core::template::ShaulaInputEnvelope,
    ) -> Result<(), TemplateOutcomeError> {
        let bytes = std::fs::read_to_string(artifact.join("profile.yaml"))
            .map_err(|_| super::state_err("input.contract"))?;
        let manifest = crate::manifest::parse_manifest(&bytes)
            .map_err(|_| super::state_err("input.contract"))?;
        input
            .validate_for_manifest(&manifest)
            .map_err(|_| super::state_err("input.contract"))
    }
    pub fn with_operation_logs(mut self, archive: Arc<dyn OperationLogSink>) -> Self {
        self.operation_logs = Some(archive);
        self
    }

    /// Call after stopping lifecycle tasks, before releasing the database ownership lock.
    pub async fn drain_operation_logs(&self) {
        self.capture_tasks.drain().await;
    }

    pub(super) async fn log_prepare(
        &self,
        workspace: &Path,
        artifact: &Path,
        digest: &str,
        timeout: Duration,
    ) -> Result<(), TemplateOutcomeError> {
        let generation = workspace
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let environment = self
            .http_backend
            .as_ref()
            .map(|backend| backend.environment(&[]))
            .transpose()
            .map_err(|_| super::state_err("backend.environment"))?
            .unwrap_or_default();
        let guard = InvocationGuard::begin(
            self.operation_logs.clone(),
            &self.capture_tasks,
            generation,
            "Create",
            SensitiveValues::from_input(&serde_json::Value::Null, &environment),
        )
        .await;
        let future = self.prepare_create_flow(workspace, artifact, digest, timeout);
        match guard {
            Some(guard) => {
                let result = guard.scope(future).await;
                guard
                    .finish(if result.is_ok() {
                        "succeeded"
                    } else {
                        "failed"
                    })
                    .await;
                result
            }
            None => future.await,
        }
    }

    pub(super) async fn log_create(
        &self,
        mut request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        self.configure_environment(&request.input.generation.id, &mut request.environment)?;
        let input = serde_json::to_value(&request.input).unwrap_or_default();
        let values = SensitiveValues::from_input(&input, &request.environment);
        let guard = InvocationGuard::begin(
            self.operation_logs.clone(),
            &self.capture_tasks,
            &request.input.generation.id,
            "Create",
            values,
        )
        .await;
        match guard {
            Some(guard) => {
                let result = guard.scope(self.create_flow(request)).await;
                guard
                    .finish(if result.is_ok() {
                        "succeeded"
                    } else {
                        "failed"
                    })
                    .await;
                result
            }
            None => self.create_flow(request).await,
        }
    }

    pub(super) async fn log_destroy(
        &self,
        mut request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.configure_environment(&request.generation_id, &mut request.environment)?;
        // This is only redaction input: execution still verifies the original protected digest.
        let input = redaction_input(&request.workspace_path).await;
        let values = match input {
            Some(input) => SensitiveValues::from_input(&input, &request.environment),
            None => SensitiveValues::withhold_all(),
        };
        let guard = InvocationGuard::begin(
            self.operation_logs.clone(),
            &self.capture_tasks,
            &request.generation_id,
            "Destroy",
            values,
        )
        .await;
        match guard {
            Some(guard) => {
                let result = guard.scope(self.destroy_flow(request)).await;
                let outcome = match &result {
                    Ok(DestroyClassification::AlreadyEmpty) => "skipped",
                    Ok(_) => "succeeded",
                    Err(_) => "failed",
                };
                guard.finish(outcome).await;
                result
            }
            None => self.destroy_flow(request).await,
        }
    }
}

async fn redaction_input(workspace: &Path) -> Option<serde_json::Value> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(workspace.join("shaula.tfvars.json"))
        .await
        .ok()?;
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    if bytes.len() > 16 * 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
