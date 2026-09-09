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
            executable: resolve(executable)?,
            workspace,
            prefix,
            deadline: Instant::now() + timeout,
        })
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
        let mut command =
            crate::engine::build_engine_command(&self.executable, self.workspace, &arguments, &[])
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

fn resolve(name: &str) -> Result<PathBuf, TemplateOutcomeError> {
    // Names are internal constants; neither Fleet nor artifact selects argv or
    // the executable. Resolve before env_clear so the child needs no PATH.
    let paths = std::env::var_os("PATH").ok_or_else(failed)?;
    #[cfg(windows)]
    let executable_name = format!("{name}.exe");
    #[cfg(windows)]
    let name = executable_name.as_str();
    for directory in std::env::split_paths(&paths).filter(|path| path.is_absolute()) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return candidate.canonicalize().map_err(|_| failed());
        }
    }
    Err(failed())
}
