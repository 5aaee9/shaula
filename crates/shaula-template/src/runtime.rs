//! The Template Runtime: single platform-neutral seam implementing the
//! core-owned `TemplateRuntimePort`. It hides artifact materialization,
//! protected files, saved-plan admission, apply classification and the
//! state-empty proof behind fixed envelopes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::plan::{admit_create_plan, parse_plan};
use shaula_core::ports::{
    DestroyClassification, PlanProvenance, StateLineage, TemplateCreateRequest,
    TemplateCreateResult, TemplateDestroyRequest, TemplateOutcomeError, TemplateRuntimePort,
};
use shaula_core::template::ShaulaResultEnvelope;

use crate::engine::TerraformFlow;
use crate::workspace;

fn plan_err(phase: &str) -> TemplateOutcomeError {
    TemplateOutcomeError::PlanFailed {
        phase: phase.to_string(),
    }
}

fn exec_err(phase: &str) -> TemplateOutcomeError {
    TemplateOutcomeError::ExecutionFailed {
        phase: phase.to_string(),
    }
}

fn state_err(phase: &str) -> TemplateOutcomeError {
    TemplateOutcomeError::StateUnavailable {
        phase: phase.to_string(),
    }
}

fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

#[path = "runtime_input.rs"]
mod input;
use input::write_protected_input;

/// Copies the materialized artifact root into the workspace (the working
/// copy the engine mutates), leaving the store pristine.
pub(crate) fn materialize_into_workspace(
    artifact_dir: &PathBuf,
    workspace: &PathBuf,
) -> CoreResult<()> {
    crate::workspace_materialize::copy_recursive(artifact_dir, workspace)
}

/// The runtime handle shared across supervisor tasks.
#[derive(Clone)]
pub struct TemplateRuntime {
    engine_executable: PathBuf,
    http_backend: Option<crate::http_backend::HttpBackendConfig>,
    operation_logs: Option<Arc<dyn shaula_core::operation_log::OperationLogSink>>,
    operation_log_reader: Option<Arc<dyn shaula_core::operation_log::OperationLogReadPort>>,
    capture_tasks: Arc<crate::operation_capture::CaptureTasks>,
}

impl TemplateRuntime {
    /// Legacy local-state / static-validation mode. Never implicitly migrate
    /// an existing Generation when the new worker backend is available.
    pub fn new(engine_executable: PathBuf) -> Self {
        Self {
            engine_executable,
            http_backend: None,
            operation_logs: None,
            operation_log_reader: None,
            capture_tasks: Arc::default(),
        }
    }

    /// New, explicitly admitted HTTP-state Generation only. The caller owns
    /// worker/GitHub gates and must select the corresponding runtime attestation.
    pub fn with_http_backend(
        engine_executable: PathBuf,
        backend: crate::http_backend::HttpBackendConfig,
    ) -> Self {
        Self {
            engine_executable,
            http_backend: Some(backend),
            operation_logs: None,
            operation_log_reader: None,
            capture_tasks: Arc::default(),
        }
    }

    async fn open_flow(&self, timeout: Duration) -> Result<TerraformFlow, TemplateOutcomeError> {
        TerraformFlow::open(self.engine_executable.clone(), timeout)
            .await
            .map_err(|e| match e.code {
                ReasonCode::TemplatePlanFailed => plan_err("create.plan"),
                _ => exec_err("create.apply"),
            })
    }
}

#[async_trait]
impl TemplateRuntimePort for TemplateRuntime {
    /// R9-07 (spec 0004 §6): materialize + LOCKED init before any remote
    /// effect — init is long, local and retryable; the JIT token minted
    /// afterwards is short-lived.
    async fn prepare_create(
        &self,
        workspace: &Path,
        artifact_dir: &Path,
        artifact_digest: &str,
        timeout: Duration,
    ) -> Result<(), TemplateOutcomeError> {
        self.log_prepare(workspace, artifact_dir, artifact_digest, timeout)
            .await
    }

    async fn create(
        &self,
        request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        self.log_create(request).await
    }
    async fn destroy(
        &self,
        request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        self.log_destroy(request).await
    }
}
#[path = "runtime_backend.rs"]
mod backend;
#[path = "runtime_bootstrap.rs"]
mod bootstrap;
#[path = "runtime_create.rs"]
mod create_flow;
#[path = "runtime_destroy.rs"]
mod destroy_flow;
#[path = "runtime_logging.rs"]
mod logging;

/// Convenience wrapper for constructing the workspace before a Create.
pub fn prepare_workspace(
    work_root: &std::path::Path,
    fleet_key: &str,
    generation_id: &str,
) -> CoreResult<PathBuf> {
    let path = workspace::generation_workspace(work_root, fleet_key, generation_id)?;
    workspace::create_workspace(&path)?;
    Ok(path)
}

/// Removes a workspace after terminal Destroy with empty-state proof.
pub fn cleanup_workspace(
    work_root: &std::path::Path,
    fleet_key: &str,
    generation_id: &str,
) -> CoreResult<()> {
    let path = workspace::generation_workspace(work_root, fleet_key, generation_id)?;
    workspace::remove_workspace(&path)
}

/// Shared handle type used across the daemon.
pub type SharedTemplateRuntime = Arc<TemplateRuntime>;
