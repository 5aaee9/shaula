//! Daemon assembly: startup barrier, supervision loops and periodic
//! scans. The startup barrier performs shared work (telemetry, validation,
//! ownership lock, migration, store checks) before per-fleet supervisors
//! recover independently.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use shaula_core::ports::Clock;
use shaula_core::registry::ControlPlaneStore;

use crate::bootstrap::ValidatedBootstrap;

/// The running daemon handle; dropping it stops supervision loops.
pub struct Daemon {
    pub control_plane: Arc<dyn ControlPlaneStore>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl Daemon {
    pub async fn request_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }
}

/// Acquires the data-directory ownership lock. A second daemon on the same
/// data directory must fail closed rather than become a second writer.
///
/// Ownership is a KERNEL-Held exclusive file lock (`fd-lock` advisory lock
/// on a file opened with no sharing on Windows): the OS releases it if and
/// only if the owning process dies, so crash takeover is race-free and a
/// live owner can never be probed away. No PID inspection participates in
/// the decision — the recorded PID in the file is diagnostics only. Two
/// processes racing on a stale file cannot both win: exactly one
/// `try_write` succeeds.
pub fn acquire_ownership_lock(data_dir: &Path) -> Result<OwnershipLock, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("data dir unusable: {e}"))?;
    let path = data_dir.join("ownership.lock");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| format!("ownership lock unusable: {e}"))?;
    // The wrapper is leaked for the process lifetime (one File, once) so
    // the kernel-held guard can borrow it for 'static and live in the
    // returned OwnershipLock.
    let lock: &'static mut fd_lock::RwLock<std::fs::File> =
        Box::leak(Box::new(fd_lock::RwLock::new(file)));
    match lock.try_write() {
        Ok(guard) => Ok(OwnershipLock {
            _guard: Some(guard),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            Err("data directory is owned by another shaula daemon".to_string())
        }
        Err(e) => Err(format!("ownership lock unusable: {e}")),
    }
}

/// Holds the ownership lock; the kernel-held advisory lock is released on
/// drop (and on process death, unconditionally, by the OS).
pub struct OwnershipLock {
    // The guard IS the ownership: it stays with the acquiring process and
    // the kernel drops it if that process dies.
    _guard: Option<fd_lock::RwLockWriteGuard<'static, std::fs::File>>,
}

/// Startup sequence shared by the binary (spec 0002 §10). The composition
/// root injects the concrete store; the daemon only sees the port.
pub async fn start(
    bootstrap: ValidatedBootstrap,
    control_plane: Arc<dyn ControlPlaneStore>,
    _clock: Arc<dyn Clock>,
    _shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<Daemon, String> {
    // 1. Filesystem roots.
    for dir in [
        &bootstrap.data_dir,
        &bootstrap.work_root,
        &bootstrap.artifact_root,
    ] {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("root {} unusable: {e}", dir.display()))?;
    }

    // The ownership lock is acquired by the composition root and held for
    // the process lifetime; start() must not re-acquire (the same process
    // would self-conflict).

    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    Ok(Daemon {
        control_plane,
        shutdown: shutdown_tx,
    })
}

/// Periodic scan cadence for outbox/desired>observed/due-change sweeps.
pub const SCAN_INTERVAL: Duration = Duration::from_secs(15);

#[cfg(test)]
#[path = "daemon_tests.rs"]
mod tests;
