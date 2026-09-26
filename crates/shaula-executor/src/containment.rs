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
