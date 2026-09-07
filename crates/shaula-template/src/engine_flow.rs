//! TerraformFlow command sequences, split to keep files within 400
//! lines (AGENTS.md).

use std::path::{Path, PathBuf};
use std::time::Duration;

use shaula_core::error::{CoreResult, ReasonCode};

use super::engine_process::EngineSpawn;
use super::engine_state::parse_raw_state_v4;
use super::{build_engine_command, spawn_fenced};
use super::{engine_version, err, hash_binary, run_engine, ProcessOutput, TerraformFlow};

impl TerraformFlow {
    pub async fn open(executable: PathBuf, timeout: Duration) -> CoreResult<Self> {
        let binary_digest = hash_binary(&executable)?;
        let version = engine_version(&executable, timeout).await?;
        Ok(Self {
            executable,
            binary_digest,
            version,
            timeout,
        })
    }

    pub async fn init(&self, cwd: &Path, env: &[(String, String)]) -> CoreResult<ProcessOutput> {
        let args = vec![
            "init".to_string(),
            "-backend=false".to_string(),
            "-input=false".to_string(),
            // R9-06 (spec 0004 §5): the published lock file IS the
            // attested provider-set authority — init must never modify
            // it; a provider drift has to fail the run, not be silently
            // upgraded into the workspace.
            "-lockfile=readonly".to_string(),
        ];
        let output = run_engine(&self.executable, cwd, &args, env, self.timeout).await?;
        self.require_success(output, "init")
    }

    pub async fn plan(
        &self,
        cwd: &Path,
        env: &[(String, String)],
        destroy: bool,
    ) -> CoreResult<ProcessOutput> {
        // The protected input must be frozen INTO the saved plan so the
        // later `apply tfplan` cannot be re-value-attacked (spec 0004 §5):
        // `shaula.tfvars.json` is not in Terraform's auto-load set.
        let mut args = vec![
            "plan".to_string(),
            "-input=false".to_string(),
            "-var-file=shaula.tfvars.json".to_string(),
            "-out=tfplan".to_string(),
        ];
        if destroy {
            args.push("-destroy".to_string());
        }
        let output = run_engine(&self.executable, cwd, &args, env, self.timeout).await?;
        self.require_success(output, "plan")
    }

    pub async fn show_plan_json(
        &self,
        cwd: &Path,
        env: &[(String, String)],
    ) -> CoreResult<serde_json::Value> {
        let args = vec![
            "show".to_string(),
            "-json".to_string(),
            "tfplan".to_string(),
        ];
        let output = run_engine(&self.executable, cwd, &args, env, self.timeout).await?;
        let output = self.require_success(output, "show")?;
        serde_json::from_str(&output.stdout).map_err(|e| {
            err(
                ReasonCode::TemplatePlanFailed,
                format!("plan JSON invalid: {e}"),
            )
        })
    }

    pub async fn apply_saved_plan(
        &self,
        cwd: &Path,
        env: &[(String, String)],
    ) -> CoreResult<ProcessOutput> {
        self.start_apply_saved_plan(cwd, env).await?.wait().await
    }

    /// STARTS the apply of the saved plan and returns the OWNED spawn
    /// (R6-03): returning means the spawn handover is complete — the
    /// caller releases the short admission claim at exactly this point
    /// and then awaits the fenced process. The child is owned by its
    /// process-tree fence from here on, never by the admission gate.
    pub async fn start_apply_saved_plan(
        &self,
        cwd: &Path,
        env: &[(String, String)],
    ) -> CoreResult<EngineSpawn> {
        let args = vec![
            "apply".to_string(),
            "-input=false".to_string(),
            "-auto-approve".to_string(),
            "tfplan".to_string(),
        ];
        let mut command = build_engine_command(&self.executable, cwd, &args, env)?;
        spawn_fenced(&mut command, self.timeout)
    }

    /// Read-only state listing for the empty-state proof. Read-only
    /// diagnosis never applies, imports or repairs.
    pub async fn state_list(
        &self,
        cwd: &Path,
        env: &[(String, String)],
    ) -> CoreResult<Vec<String>> {
        let args = vec!["state".to_string(), "list".to_string()];
        let output = run_engine(&self.executable, cwd, &args, env, self.timeout).await?;
        if !output.success() {
            // No state at all counts as empty; anything else fails closed.
            // Terraform v1.9's official fresh-workspace diagnostic is
            // "No state file was found!" — matched case-insensitively
            // against the stable "no state" phrase, never guessed from
            // unrelated errors.
            let stderr = output.stderr.to_lowercase();
            if stderr.contains("no state") || stderr.contains("empty") {
                return Ok(Vec::new());
            }
            return Err(err(
                ReasonCode::TemplateExecutionFailed,
                "state list failed",
            ));
        }
        Ok(output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// The REAL state identity from `terraform state pull` (read-only):
    /// managed resource addresses (data sources are NOT deletion targets),
    /// the serial and the lineage — used for the destroy provenance and
    /// exact managed-state coverage (spec 0004 §5).
    ///
    /// `state pull` emits the RAW state (format version 4): root-module
    /// resources live at the TOP LEVEL and child modules under
    /// `root_module.child_modules`, with per-resource `instances`
    /// (count/for_each). The `terraform show -json` shape (`values.root_
    /// module`) is a DIFFERENT document and is rejected — reading it as
    /// "empty" would pass Destroy over live resources (F01).
    pub async fn state_pull_snapshot(
        &self,
        cwd: &Path,
        env: &[(String, String)],
    ) -> CoreResult<crate::workspace::StateSnapshot> {
        let args = vec!["state".to_string(), "pull".to_string()];
        let output = run_engine(&self.executable, cwd, &args, env, self.timeout).await?;
        if !output.success() {
            return Err(err(
                ReasonCode::TemplateExecutionFailed,
                "state pull failed",
            ));
        }
        let state: serde_json::Value = serde_json::from_str(&output.stdout).map_err(|e| {
            err(
                ReasonCode::TemplateExecutionFailed,
                format!("state pull JSON invalid: {e}"),
            )
        })?;
        parse_raw_state_v4(&state)
    }

    pub async fn output_json(
        &self,
        cwd: &Path,
        env: &[(String, String)],
    ) -> CoreResult<serde_json::Value> {
        let args = vec!["output".to_string(), "-json".to_string()];
        let output = run_engine(&self.executable, cwd, &args, env, self.timeout).await?;
        if !output.success() {
            // No outputs yet is a valid intermediate state.
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&output.stdout).map_err(|e| {
            err(
                ReasonCode::TemplateExecutionFailed,
                format!("output JSON invalid: {e}"),
            )
        })
    }

    pub(crate) fn require_success(
        &self,
        output: ProcessOutput,
        phase: &str,
    ) -> CoreResult<ProcessOutput> {
        if output.success() {
            Ok(output)
        } else if phase == "plan" || phase == "show" {
            Err(err(
                ReasonCode::TemplatePlanFailed,
                format!("terraform {phase} failed"),
            ))
        } else {
            Err(err(
                ReasonCode::TemplateExecutionFailed,
                format!("terraform {phase} failed"),
            ))
        }
    }
}

/// Extracts the `shaula_result` output value from `terraform output -json`.
pub fn extract_shaula_result(outputs: &serde_json::Value) -> CoreResult<&serde_json::Value> {
    outputs
        .get("shaula_result")
        .and_then(|o| o.get("value"))
        .ok_or_else(|| {
            err(
                ReasonCode::TemplateExecutionFailed,
                "shaula_result output missing",
            )
        })
}
