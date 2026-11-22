//! Read-only failed-Create discovery and targeted import (ARD-0044).
//! Runs `discover` with `-backend=false` in an isolated private temp dir,
//! verifies zero managed plan actions, and only imports remote resources
//! explicitly returned by `shaula_discover` with a non-null `import_id`.

use shaula_core::plan::{admit_discover_plan, parse_plan};
use shaula_core::ports::{ReconcileClassification, TemplateOutcomeError, TemplateReconcileRequest};
use shaula_core::template::DiscoverOutput;

use super::{digest_of, exec_err, plan_err, state_err, TemplateRuntime};

impl TemplateRuntime {
    pub(super) async fn reconcile_flow(
        &self,
        mut request: TemplateReconcileRequest,
    ) -> Result<ReconcileClassification, TemplateOutcomeError> {
        let workspace = &request.workspace_path;
        self.verify_http_workspace(workspace, true)?;
        let flow = self.open_flow(request.timeout).await?;
        let original = &request.original_provenance;

        // Exact Create provenance re-verification (identical to destroy_flow).
        let input_now = std::fs::read(workspace.join("shaula.tfvars.json"))
            .map_err(|_| state_err("reconcile.input"))?;
        if digest_of(&input_now) != original.protected_input_digest {
            return Err(state_err("reconcile.input"));
        }
        let input_val: serde_json::Value = serde_json::from_slice(&input_now)
            .map_err(|_| state_err("reconcile.input_contract"))?;
        let input = serde_json::from_value(input_val.get("shaula").cloned().unwrap_or_default())
            .map_err(|_| state_err("reconcile.input_contract"))?;
        let manifest = self.validate_input_contract(&request.artifact_dir, &input)?;
        let Some(declared) = &manifest.reconcile else {
            return Err(plan_err("reconcile.unsupported"));
        };
        if declared.discover_module != request.discover_module {
            return Err(plan_err("reconcile.contract"));
        }
        crate::reconcile_validate::verify_reconcile_module(&request.artifact_dir, &manifest)
            .map_err(|_| plan_err("reconcile.contract"))?;

        let workspace_now = self
            .workspace_digest(workspace)
            .await
            .map_err(|_| state_err("reconcile.material"))?;
        let expected_material = crate::artifact_integrity::material_digest_off_thread(
            &request.artifact_dir,
            &request.pinned_artifact_digest,
        )
        .await
        .map_err(|_| state_err("reconcile.material"))?;
        if !crate::artifact_integrity::provenance_matches(
            original,
            &request.pinned_artifact_digest,
            &expected_material,
        ) || workspace_now != expected_material
        {
            return Err(state_err("reconcile.material"));
        }
        if flow.binary_digest != original.engine_binary_digest {
            return Err(exec_err("reconcile.engine"));
        }

        let snapshot = flow
            .state_pull_snapshot(workspace, &request.environment)
            .await
            .map_err(|_| state_err("reconcile.state"))?;

        let ssh = crate::docker_connection::DockerSsh::prepare(&manifest, &input.bindings)
            .map_err(|_| plan_err("reconcile.plan"))?;
        if let Some(ssh) = &ssh {
            ssh.configure(&mut request.environment);
        }

        // Materialize ONLY the validated read-only discover module, locked
        // provider lockfile, and retained tfvars into a private scratch dir
        // with NO HTTP state backend variables.
        let scratch = crate::workspace::private_tempdir("shaula-discover-")
            .map_err(|_| state_err("reconcile.scratch"))?;
        crate::workspace_materialize::copy_recursive(
            &request.artifact_dir.join(&declared.discover_module),
            scratch.path(),
        )
        .map_err(|_| state_err("reconcile.scratch"))?;
        std::fs::copy(
            request.artifact_dir.join(".terraform.lock.hcl"),
            scratch.path().join(".terraform.lock.hcl"),
        )
        .map_err(|_| state_err("reconcile.scratch"))?;
        std::fs::write(scratch.path().join("shaula.tfvars.json"), &input_now)
            .map_err(|_| state_err("reconcile.scratch"))?;

        let discover_env: Vec<(String, String)> = request
            .environment
            .iter()
            .filter(|(k, _)| !k.starts_with("TF_HTTP_"))
            .cloned()
            .collect();

        flow.init(scratch.path(), &discover_env)
            .await
            .map_err(|_| exec_err("reconcile.init"))?;
        flow.plan_discover(scratch.path(), &discover_env)
            .await
            .map_err(|_| exec_err("reconcile.discover"))?;
        let plan_json = flow
            .show_named_plan_json(scratch.path(), &discover_env, "discover.tfplan")
            .await
            .map_err(|_| plan_err("reconcile.show"))?;
        let parsed = parse_plan(&plan_json).map_err(|_| plan_err("reconcile.plan"))?;
        admit_discover_plan(&parsed).map_err(|_| plan_err("reconcile.plan"))?;

        let raw_output = plan_json
            .pointer("/planned_values/outputs/shaula_discover/value")
            .ok_or_else(|| plan_err("reconcile.output"))?;
        let discovered: DiscoverOutput =
            serde_json::from_value(raw_output.clone()).map_err(|_| plan_err("reconcile.output"))?;
        discovered
            .validate(&request.generation_id, &request.managed_shape)
            .map_err(|_| plan_err("reconcile.output"))?;

        let untracked: Vec<_> = discovered
            .all_resources()
            .into_iter()
            .filter(|resource| {
                !snapshot
                    .managed
                    .iter()
                    .any(|addr| addr == &resource.address)
            })
            .collect();

        if untracked.is_empty() {
            if snapshot.managed.is_empty() {
                return Ok(ReconcileClassification::Empty {
                    state_lineage: snapshot.lineage,
                    state_serial: snapshot.serial,
                });
            }
            return Ok(ReconcileClassification::Anchored {
                state_lineage: snapshot.lineage,
                state_serial: snapshot.serial,
                adopted_count: 0,
            });
        }

        // Fail closed if ANY untracked remote resource cannot be imported.
        for resource in &untracked {
            if resource.import_id.is_none() {
                return Ok(ReconcileClassification::Unimportable {
                    address: resource.address.clone(),
                });
            }
        }

        for resource in &untracked {
            let import_id = resource
                .import_id
                .as_deref()
                .ok_or_else(|| plan_err("reconcile.import_id"))?;
            flow.import_resource(
                workspace,
                &request.environment,
                &resource.address,
                import_id,
            )
            .await
            .map_err(|_| exec_err("reconcile.import"))?;
        }

        let after = flow
            .state_pull_snapshot(workspace, &request.environment)
            .await
            .map_err(|_| state_err("reconcile.state"))?;
        if after.lineage != snapshot.lineage || after.serial < snapshot.serial {
            return Err(state_err("reconcile.state"));
        }
        if !untracked
            .iter()
            .all(|resource| after.managed.iter().any(|addr| addr == &resource.address))
        {
            return Err(exec_err("reconcile.import_verify"));
        }

        Ok(ReconcileClassification::Anchored {
            state_lineage: after.lineage,
            state_serial: after.serial,
            adopted_count: untracked.len(),
        })
    }
}
