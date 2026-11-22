//! Fixed CLI execution under the same process-tree fence as Terraform.
use super::{failed, TemplateOutcomeError};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) struct Commands<'a> {
    executable: PathBuf,
    workspace: &'a Path,
    prefix: Vec<String>,
    environment: Vec<(String, String)>,
    deadline: Instant,
}

impl<'a> Commands<'a> {
    pub(super) fn new(
        executable: &str,
        workspace: &'a Path,
        prefix: Vec<String>,
        timeout: Duration,
    ) -> Result<Self, TemplateOutcomeError> {
        Ok(Self {
            executable: crate::docker_connection::resolve_executable(executable)
                .map_err(|_| failed())?,
            workspace,
            prefix,
            environment: Vec::new(),
            deadline: Instant::now() + timeout,
        })
    }

    pub(super) fn with_environment(mut self, environment: Vec<(String, String)>) -> Self {
        self.environment = environment;
        self
    }

    pub(super) async fn run(&self, args: &[&str]) -> Result<String, TemplateOutcomeError> {
        self.run_bounded(args, Duration::from_secs(15)).await
    }

    pub(super) async fn optional(&self, args: &[&str]) {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let budget = optional_budget(remaining);
        if !budget.is_zero() {
            // Bound the complete optional command, including pipe draining.
            // Cancellation drops the existing process-tree fence.
            let _ = tokio::time::timeout(budget, self.run_bounded(args, budget)).await;
        }
    }

    async fn run_bounded(
        &self,
        args: &[&str],
        limit: Duration,
    ) -> Result<String, TemplateOutcomeError> {
        let timeout = self.deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(failed());
        }
        let mut arguments = self.prefix.clone();
        arguments.extend(args.iter().map(|value| (*value).to_string()));
        let mut command = crate::engine::build_engine_command(
            &self.executable,
            self.workspace,
            &arguments,
            &self.environment,
        )
        .map_err(|_| failed())?;
        let output = crate::engine::engine_process::spawn_fenced_logged(
            &mut command,
            timeout.min(limit),
            None,
        )
        .map_err(|_| failed())?
        .wait()
        .await
        .map_err(|_| failed())?;
        if !output.success() {
            return Err(failed());
        }
        Ok(output.stdout)
    }

    pub(super) async fn json(
        &self,
        args: &[&str],
    ) -> Result<serde_json::Value, TemplateOutcomeError> {
        serde_json::from_str(&self.run(args).await?).map_err(|_| failed())
    }
}

pub(super) fn optional_budget(remaining: Duration) -> Duration {
    // Preserve both mandatory commands that follow the diagnostic copy:
    // one exact identity re-inspection and one container start.
    remaining
        .saturating_sub(Duration::from_secs(30))
        .min(Duration::from_secs(5))
}
