//! Explicit backend setup shared by Create and Destroy. Old runtime callers
//! keep local state; opting in never overwrites/migrates an existing workspace.

use std::{path::Path, time::Duration};

use crate::engine::TerraformFlow;
use shaula_core::{error::CoreResult, ports::TemplateOutcomeError};

use super::{exec_err, materialize_into_workspace, state_err, TemplateRuntime};

impl TemplateRuntime {
    pub(super) fn configure_environment(
        &self,
        generation: &str,
        environment: &mut Vec<(String, String)>,
    ) -> Result<(), TemplateOutcomeError> {
        if let Some(backend) = &self.http_backend {
            backend
                .check_generation(generation)
                .map_err(|_| state_err("backend.identity"))?;
            *environment = backend
                .environment(environment)
                .map_err(|_| state_err("backend.environment"))?;
        }
        Ok(())
    }

    /// Template-material digest of the workspace, computed on the blocking
    /// pool (it walks and hashes every template file).
    pub(super) async fn workspace_digest(&self, workspace: &Path) -> CoreResult<String> {
        let backend = self.http_backend.clone();
        let workspace = workspace.to_path_buf();
        crate::engine::off_thread(move || match &backend {
            Some(backend) => {
                backend.verify(&workspace, false)?;
                crate::workspace::http_template_files_digest(&workspace)
            }
            None => crate::workspace::template_files_digest(&workspace),
        })
        .await
    }

    pub(super) fn require_fresh_http_workspace(
        &self,
        workspace: &Path,
    ) -> Result<(), TemplateOutcomeError> {
        if self.http_backend.is_some() {
            crate::http_backend::fresh_workspace(workspace)
                .map_err(|_| state_err("backend.workspace"))?;
        }
        Ok(())
    }

    pub(super) fn verify_http_workspace(
        &self,
        workspace: &Path,
        initialized: bool,
    ) -> Result<(), TemplateOutcomeError> {
        if let Some(backend) = &self.http_backend {
            backend
                .verify(workspace, initialized)
                .map_err(|_| state_err("backend.workspace"))?;
        }
        Ok(())
    }

    pub(super) async fn initialize_workspace(
        &self,
        flow: &TerraformFlow,
        workspace: &Path,
        environment: &[(String, String)],
    ) -> Result<(), TemplateOutcomeError> {
        if self.http_backend.is_some() {
            flow.init_http(workspace, environment).await
        } else {
            flow.init(workspace, environment).await
        }
        .map_err(|_| exec_err("create.init"))?;
        self.verify_http_workspace(workspace, true)
    }

    pub(super) async fn prepare_create_flow(
        &self,
        workspace: &Path,
        artifact_dir: &Path,
        artifact_digest: &str,
        timeout: Duration,
    ) -> Result<(), TemplateOutcomeError> {
        let manifest = std::fs::read_to_string(artifact_dir.join("profile.yaml"))
            .map_err(|_| state_err("input.contract"))?;
        crate::manifest::parse_manifest(&manifest)
            .and_then(|manifest| manifest.validate_new_container_profile())
            .map_err(|_| state_err("input.contract"))?;
        let expected =
            crate::artifact_integrity::material_digest_off_thread(artifact_dir, artifact_digest)
                .await
                .map_err(|_| state_err("create.materialize"))?;
        self.require_fresh_http_workspace(workspace)?;
        materialize_into_workspace(artifact_dir, workspace)
            .map_err(|_| state_err("create.materialize"))?;
        let materialized = workspace.to_path_buf();
        if crate::engine::off_thread(move || crate::workspace::template_files_digest(&materialized))
            .await
            .map_err(|_| state_err("create.materialize"))?
            != expected
        {
            return Err(state_err("create.materialize"));
        }
        let environment = match &self.http_backend {
            Some(backend) => {
                backend
                    .install(workspace)
                    .map_err(|_| state_err("create.backend"))?;
                backend
                    .environment(&[])
                    .map_err(|_| state_err("backend.environment"))?
            }
            None => Vec::new(),
        };
        let flow = self.open_flow(timeout).await?;
        self.initialize_workspace(&flow, workspace, &environment)
            .await
    }
}
