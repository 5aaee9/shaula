//! Terraform engine execution: argument-vector subprocess with explicit
//! environment allowlist, workspace cwd, binary hashing, bounded output and
//! timeouts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

use engine_process::spawn_fenced;

#[path = "engine_process.rs"]
pub mod engine_process;

#[path = "engine_supervisor.rs"]
pub mod engine_supervisor;

const MAX_OUTPUT_BYTES: usize = 1 << 20; // 1 MiB per stream

pub(crate) fn err(code: ReasonCode, msg: impl Into<String>) -> CoreError {
    CoreError::new(code, msg)
}

/// Bounded stdout/stderr capture from a finished child.
#[derive(Debug, Clone, Default)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ProcessOutput {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Bounded-prefix pipe reader: keeps the first [`MAX_OUTPUT_BYTES`]
/// bytes, always drains to EOF and reports I/O errors instead of
/// silently yielding empty output. Each stream owns ONE task running
/// this helper (two tasks, two buffers — never merged, C12).
pub(crate) async fn bounded_drain<R: tokio::io::AsyncRead + Unpin>(
    mut pipe: R,
) -> std::io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Err(e) => return Err(e),
            Ok(n) => {
                if kept.len() < MAX_OUTPUT_BYTES {
                    let take = n.min(MAX_OUTPUT_BYTES - kept.len());
                    kept.extend_from_slice(&chunk[..take]);
                }
            }
        }
    }
    Ok(kept)
}

/// Builds one engine command. The environment is always rebuilt from an
/// explicit allowlist; the daemon environment is never forwarded.
///
/// R7-06/R9-09: EVERY engine invocation is routed behind the fence
/// supervisor (`engine_supervisor`) — on Windows the engine is a BORN
/// member of a kill-on-close job; on Unix the supervisor is its own
/// process-group leader and watches for daemon death, so a SIGKILLed
/// daemon still terminates the whole tree.
pub(crate) fn build_engine_command(
    executable: &Path,
    cwd: &Path,
    args: &[String],
    extra_env: &[(String, String)],
) -> CoreResult<Command> {
    let supervisor = engine_supervisor::supervisor_executable()?;
    let mut command = Command::new(supervisor);
    command
        .arg(engine_supervisor::FENCE_ARG)
        .arg(executable)
        .args(args);
    // stdin is PIPED: the daemon-held write end is the supervisor's
    // daemon-death signal. The supervisor's env BECOMES the engine's env
    // (it spawns with inherit).
    command.stdin(std::process::Stdio::piped());
    command
        .current_dir(cwd)
        .env_clear()
        // A timed-out OR cancelled engine must not outlive the caller:
        // the drop of the child handle kills the process (R5-05); the
        // process-tree fence covers descendants (R6-04, R7-06, R9-09).
        .kill_on_drop(true)
        .env("TF_IN_AUTOMATION", "1")
        .env("TF_INPUT", "0");
    for (key, value) in base_allowlist(extra_env) {
        command.env(key, value);
    }
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    Ok(command)
}

/// Executes one engine command to completion. The child runs under a
/// process-tree fence with abort-on-drop readers (R6-04); the spawned
/// form [`engine_process::EngineSpawn`] exists so the mutating applies
/// can release the admission claim at the spawn handover (R6-03).
pub async fn run_engine(
    executable: &Path,
    cwd: &Path,
    args: &[String],
    extra_env: &[(String, String)],
    timeout: Duration,
) -> CoreResult<ProcessOutput> {
    let mut command = build_engine_command(executable, cwd, args, extra_env)?;
    spawn_fenced(&mut command, timeout)?.wait().await
}

fn base_allowlist(extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut allowed = vec![
        ("TF_IN_AUTOMATION".to_string(), "1".to_string()),
        ("TF_INPUT".to_string(), "0".to_string()),
    ];
    // Windows needs SystemRoot for the crypto/network stack of the child.
    if let Ok(system_root) = std::env::var("SystemRoot") {
        allowed.push(("SystemRoot".to_string(), system_root));
    }
    if let Ok(tmp) = std::env::var("TEMP") {
        allowed.push(("TEMP".to_string(), tmp));
    }
    if let Ok(tmp) = std::env::var("TMP") {
        allowed.push(("TMP".to_string(), tmp));
    }
    for (key, value) in extra {
        allowed.push((key.clone(), value.clone()));
    }
    allowed
}

/// SHA-256 of the engine binary; re-verified immediately before spawn.
pub(crate) fn hash_binary(executable: &Path) -> CoreResult<String> {
    let bytes = std::fs::read(executable).map_err(|e| {
        err(
            ReasonCode::Internal,
            format!("engine binary unreadable: {e}"),
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

/// Resolves the engine version string (`terraform version`).
pub(crate) async fn engine_version(executable: &Path, timeout: Duration) -> CoreResult<String> {
    let temp = std::env::temp_dir();
    let output = run_engine(executable, &temp, &["version".to_string()], &[], timeout).await?;
    if !output.success() {
        return Err(err(ReasonCode::Internal, "engine version probe failed"));
    }
    Ok(output
        .stdout
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches("Terraform v")
        .to_string())
}

/// Terraform command sequence builder with the fixed protocol.
pub struct TerraformFlow {
    pub executable: PathBuf,
    pub binary_digest: String,
    pub version: String,
    pub timeout: Duration,
}

#[path = "engine_flow.rs"]
pub mod engine_flow;
pub use engine_flow::extract_shaula_result;

#[path = "engine_state.rs"]
pub mod engine_state;
pub use engine_process::EngineSpawn;

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
