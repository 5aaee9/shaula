use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleConfig {
    pub executor: String,
    #[serde(default = "listen")]
    pub internal_listen: SocketAddr,
    pub max_workers: usize,
    pub recovery_reserve: usize,
    /// Omit under systemd: use this service's delegated cgroup v2 directory.
    pub cgroup_root: Option<PathBuf>,
}
fn listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}
impl LifecycleConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.executor != "exec"
            || !self.internal_listen.ip().is_loopback()
            || self.recovery_reserve == 0
            || self.max_workers > 1024
            || self.recovery_reserve >= self.max_workers
            || self
                .cgroup_root
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
        {
            return Err(
                "invalid lifecycle executor, loopback address, worker limits or cgroup root".into(),
            );
        }
        Ok(())
    }
}
