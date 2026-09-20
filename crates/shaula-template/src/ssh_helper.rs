//! Internal re-exec helpers for Docker's OpenSSH transport. The aliases live
//! in an invocation-private directory and inherit the existing process fence.

use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf};

pub(crate) const CONFIG_ENV: &str = "SHAULA_DOCKER_SSH_CONFIG";

#[derive(Deserialize, Serialize)]
pub(crate) struct Config {
    pub(crate) executable: PathBuf,
    pub(crate) arguments: Vec<String>,
    pub(crate) secret_file: Option<PathBuf>,
}

/// Dispatch before CLI parsing/logging. Neither SSH transport bytes nor
/// askpass responses may pass through the ordinary Shaula CLI or telemetry.
/// Returns `None` when invoked as the normal Shaula executable.
#[doc(hidden)]
pub fn dispatch() -> Option<i32> {
    let executable = PathBuf::from(std::env::args_os().next()?);
    let name = executable.file_stem()?.to_str()?;
    if !matches!(name, "ssh" | "shaula-ssh-askpass") {
        return None;
    }
    Some(run(name).unwrap_or(1))
}

fn run(name: &str) -> std::io::Result<i32> {
    let path = std::env::var_os(CONFIG_ENV)
        .ok_or_else(|| std::io::Error::other("SSH helper configuration missing"))?;
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    if name == "shaula-ssh-askpass" {
        let path = config
            .secret_file
            .ok_or_else(|| std::io::Error::other("SSH credential missing"))?;
        let secret = zeroize::Zeroizing::new(std::fs::read(path)?);
        let mut output = std::io::stdout().lock();
        output.write_all(&secret)?;
        output.write_all(b"\n")?;
        output.flush()?;
        return Ok(0);
    }
    let mut command = std::process::Command::new(config.executable);
    command
        .args(config.arguments)
        .args(std::env::args_os().skip(1));
    // Preserve Docker's bidirectional dial-stdio pipes. No shell and no secret
    // argv/environment; OpenSSH obtains passwords through the private helper.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec())
    }
    #[cfg(not(unix))]
    {
        Ok(command.status()?.code().unwrap_or(1))
    }
}
