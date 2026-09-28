//! Linux cgroup v2 provides a kernel-owned descendant set across nested
//! Terraform process groups and daemon death. Unsupported hosts fail closed.
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::ProcessIdentity,
};
use std::path::{Path, PathBuf};

pub(crate) fn root(path: &Path) -> StateResult<PathBuf> {
    if !cfg!(target_os = "linux") {
        return Err(StateError::Unavailable);
    }
    let path = std::fs::canonicalize(path).map_err(|_| StateError::Unavailable)?;
    if !path.starts_with("/sys/fs/cgroup") || !path.join("cgroup.controllers").is_file() {
        return Err(StateError::Invalid);
    }
    Ok(path)
}

pub(crate) fn boot() -> StateResult<String> {
    let machine =
        std::fs::read_to_string("/etc/machine-id").map_err(|_| StateError::Unavailable)?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| StateError::Unavailable)?;
    Ok(format!("{}:{}", machine.trim(), boot.trim()))
}

pub(crate) fn started(pid: u32) -> StateResult<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| StateError::Unavailable)?;
    // The parenthesized comm may contain whitespace and ')'. Field 22 is the
    // kernel start time; field 3 starts immediately after the final ')'.
    let (_, tail) = stat.rsplit_once(')').ok_or(StateError::Unavailable)?;
    // A dead worker can remain as an unreaped zombie while a detached provider
    // still populates its cgroup. Its matching PID/start time is not liveness.
    if matches!(tail.split_whitespace().next(), Some("Z" | "X" | "x") | None) {
        return Err(StateError::Unavailable);
    }
    tail.split_whitespace()
        .nth(19)
        .filter(|value| value.bytes().all(|c| c.is_ascii_digit()))
        .map(str::to_owned)
        .ok_or(StateError::Unavailable)
}

pub(crate) fn path(root: &Path, identity: &ProcessIdentity) -> StateResult<PathBuf> {
    identity.validate()?;
    let recorded = Path::new(&identity.containment);
    if recorded.parent() != Some(root) {
        return Err(StateError::Invalid);
    }
    let id = recorded
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StateError::Invalid)?
        .strip_prefix("shaula-")
        .ok_or(StateError::Invalid)?;
    let id = uuid::Uuid::parse_str(id).map_err(|_| StateError::Invalid)?;
    let path = root.join(format!("shaula-{id}"));
    if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(StateError::Invalid);
    }
    Ok(path)
}

pub(crate) fn empty(path: &Path) -> StateResult<bool> {
    let events =
        std::fs::read_to_string(path.join("cgroup.events")).map_err(|_| StateError::Unavailable)?;
    if events.lines().any(|line| line == "populated 0") {
        Ok(true)
    } else if events.lines().any(|line| line == "populated 1") {
        Ok(false)
    } else {
        Err(StateError::Unavailable)
    }
}

/// Kills every member of `path` and waits for the kernel to report it empty.
/// A successful kill write alone is not a fence.
pub(crate) async fn kill(path: &Path) -> bool {
    if !empty(path).is_ok_and(|empty| empty)
        && std::fs::write(path.join("cgroup.kill"), "1").is_err()
    {
        return false;
    }
    for _ in 0..100 {
        if empty(path).is_ok_and(|empty| empty) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    false
}

/// cgroup v2 directory inode: the kernel's per-boot, never-reused cgroup ID.
pub(crate) fn root_id(root: &Path) -> StateResult<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(root)
            .map(|meta| meta.ino())
            .map_err(|_| StateError::Unavailable)
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err(StateError::Unavailable)
    }
}

/// Whether the recorded tree provably ended without inspecting its group:
/// a reboot of the same host, or on the same boot a replaced delegated root.
/// systemd removes a delegated unit subtree after control-group termination;
/// the kernel removes a cgroup only once every descendant is empty, and the
/// service user cannot move processes above its delegated root. The caller
/// must first bind the identity to this root with [`path`].
pub(crate) fn ended(root: &Path, identity: &ProcessIdentity) -> StateResult<bool> {
    if rebooted(identity)? {
        return Ok(true);
    }
    match identity.root_id {
        Some(recorded) => Ok(root_id(root)? != recorded),
        None => Ok(false),
    }
}

/// Only a reboot of the SAME host proves old local processes cannot execute.
pub(crate) fn rebooted(identity: &ProcessIdentity) -> StateResult<bool> {
    let current = boot()?;
    let (machine, boot) = current.split_once(':').ok_or(StateError::Unavailable)?;
    let (old_machine, old_boot) = identity
        .host_boot
        .split_once(':')
        .ok_or(StateError::Invalid)?;
    if machine != old_machine {
        return Err(StateError::Unavailable);
    }
    Ok(boot != old_boot)
}
