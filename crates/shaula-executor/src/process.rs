use sha2::{Digest, Sha256};
use shaula_core::{
    state_backend::{StateClaim, StateError, StateResult},
    worker::{
        wire::LaunchEnvelope, Executor, FenceOutcome, ProcessIdentity, ProcessObservation,
        MAX_CONTROL_BYTES,
    },
};
use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, ChildStdin, Command},
    sync::Mutex,
};
use uuid::Uuid;

use crate::containment;

struct Process {
    child: Child,
    handoff: Option<ChildStdin>,
    identity: ProcessIdentity,
    handed_off: bool,
}

pub struct ExecExecutor {
    executable: PathBuf,
    executable_digest: String,
    root: PathBuf,
    children: Mutex<HashMap<Uuid, Process>>,
}

impl ExecExecutor {
    pub fn delegated_root() -> StateResult<PathBuf> {
        let groups =
            std::fs::read_to_string("/proc/self/cgroup").map_err(|_| StateError::Unavailable)?;
        let group = groups
            .lines()
            .find_map(|line| line.strip_prefix("0::/"))
            .ok_or(StateError::Unavailable)?;
        if group.is_empty() || group.split('/').any(|part| part == "..") {
            return Err(StateError::Invalid);
        }
        containment::root(&PathBuf::from("/sys/fs/cgroup").join(group))
    }
    pub fn new(executable: PathBuf, root: PathBuf) -> StateResult<Self> {
        let executable = std::fs::canonicalize(executable).map_err(|_| StateError::Unavailable)?;
        let digest = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                std::fs::read(&executable).map_err(|_| StateError::Unavailable)?
            ))
        );
        let root = containment::root(&root)?;
        if !Self::delegated_root()?.starts_with(&root) {
            return Err(StateError::Invalid);
        }
        let probe = root.join(format!("shaula-probe-{}", Uuid::new_v4()));
        std::fs::create_dir(&probe).map_err(|_| StateError::Unavailable)?;
        let supported = probe.join("cgroup.kill").is_file();
        std::fs::remove_dir(&probe).map_err(|_| StateError::Unavailable)?;
        if !supported {
            return Err(StateError::Unavailable);
        }
        Ok(Self {
            executable,
            executable_digest: digest,
            root,
            children: Mutex::new(HashMap::new()),
        })
    }

    pub fn executable_digest(&self) -> &str {
        &self.executable_digest
    }
}

#[async_trait::async_trait]
impl Executor for ExecExecutor {
    async fn release_fenced(&self, identity: &ProcessIdentity) -> StateResult<()> {
        if containment::rebooted(identity)? {
            return Ok(());
        }
        let path = containment::path(&self.root, identity)?;
        if !path.try_exists().map_err(|_| StateError::Unavailable)? {
            return Ok(());
        }
        if !containment::empty(&path)? {
            return Err(StateError::Conflict);
        }
        std::fs::remove_dir(path).map_err(|_| StateError::Unavailable)
    }
    async fn launch(&self, claim: &StateClaim) -> StateResult<ProcessIdentity> {
        let mut children = self.children.lock().await;
        if children.contains_key(&claim.worker_attempt) {
            return Err(StateError::Conflict);
        }
        let containment = format!("shaula-{}", claim.worker_attempt);
        let group = self.root.join(&containment);
        // create_dir fails if a prior attempt left this exact containment. Its
        // absence from the in-memory map never makes a relaunch safe.
        std::fs::create_dir(&group).map_err(|_| StateError::Conflict)?;
        if !group.join("cgroup.kill").is_file() {
            return Err(StateError::Unavailable);
        }
        let mut command = Command::new(&self.executable);
        command
            .arg("job")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Only nonsecret process basics; never forward the daemon environment.
        for name in [
            "PATH",
            "HOME",
            "TMPDIR",
            "LANG",
            "SSL_CERT_FILE",
            "NIX_SSL_CERT_FILE",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => {
                let _ = std::fs::remove_dir(&group);
                return Err(StateError::Unavailable);
            }
        };
        // job cannot execute effects before reading the envelope and completing
        // its daemon handshake. Assign before releasing that inherited pipe.
        let ownership = (|| {
            let pid = child.id().ok_or(StateError::Unavailable)?;
            std::fs::write(group.join("cgroup.procs"), pid.to_string())
                .map_err(|_| StateError::Unavailable)?;
            let identity = ProcessIdentity {
                host_boot: containment::boot()?,
                process_id: pid,
                started: containment::started(pid)?,
                containment: group.to_str().ok_or(StateError::Invalid)?.to_owned(),
                root_id: Some(containment::root_id(&self.root)?),
            };
            let handoff = child.stdin.take().ok_or(StateError::Unavailable)?;
            Ok((identity, handoff))
        })();
        let (identity, handoff) = match ownership {
            Ok(ownership) => ownership,
            Err(error) => {
                // No launch envelope has been sent, so job has no authority to
                // spawn descendants. Reap the direct child even if assignment
                // or identity capture failed, instead of leaving a zombie.
                let _ = child.kill().await;
                let _ = child.wait().await;
                if containment::empty(&group).is_ok_and(|empty| empty) {
                    let _ = std::fs::remove_dir(&group);
                }
                return Err(error);
            }
        };
        children.insert(
            claim.worker_attempt,
            Process {
                child,
                handoff: Some(handoff),
                identity: identity.clone(),
                handed_off: false,
            },
        );
        Ok(identity)
    }

    async fn handoff(&self, envelope: LaunchEnvelope) -> StateResult<()> {
        envelope.validate()?;
        if envelope.executable_digest != self.executable_digest {
            return Err(StateError::Conflict);
        }
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(&envelope).map_err(|_| StateError::Invalid)?,
        );
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(StateError::TooLarge);
        }
        let mut children = self.children.lock().await;
        let process = children
            .get_mut(&envelope.claim.worker_attempt)
            .ok_or(StateError::Conflict)?;
        if process.handed_off {
            return Err(StateError::Conflict);
        }
        process.handed_off = true;
        let pipe = process.handoff.as_mut().ok_or(StateError::Conflict)?;
        tokio::time::timeout(Duration::from_secs(5), async {
            pipe.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
            pipe.write_all(&bytes).await?;
            pipe.flush().await
        })
        .await
        .map_err(|_| StateError::Unavailable)?
        .map_err(|_| StateError::Unavailable)?;
        // Keep the pipe open as the parent's lifetime signal. The flag above
        // also prohibits retry after a partial/uncertain write.
        Ok(())
    }

    async fn observe(&self, identity: &ProcessIdentity) -> ProcessObservation {
        let Ok(path) = containment::path(&self.root, identity) else {
            return ProcessObservation::Unknown;
        };
        match containment::ended(&self.root, identity) {
            Ok(true) => return ProcessObservation::Exited,
            Err(_) => return ProcessObservation::Unknown,
            Ok(false) => {}
        }
        match containment::empty(&path) {
            Ok(true) => ProcessObservation::Exited,
            Ok(false)
                if containment::started(identity.process_id)
                    .is_ok_and(|start| start == identity.started) =>
            {
                ProcessObservation::Live
            }
            _ => ProcessObservation::Unknown,
        }
    }

    async fn stop_and_fence(&self, identity: &ProcessIdentity) -> FenceOutcome {
        let Ok(path) = containment::path(&self.root, identity) else {
            return FenceOutcome::Unknown;
        };
        match containment::ended(&self.root, identity) {
            Ok(true) => return FenceOutcome::Fenced,
            Err(_) => return FenceOutcome::Unknown,
            Ok(false) => {}
        }
        if !containment::kill(&path).await {
            return FenceOutcome::Unknown;
        }
        let mut children = self.children.lock().await;
        let attempt = children
            .iter()
            .find(|(_, p)| p.identity == *identity)
            .map(|(id, _)| *id);
        if let Some(attempt) = attempt {
            if let Some(mut process) = children.remove(&attempt) {
                let _ = process.child.wait().await;
            }
        }
        // Keep the empty cgroup as restart-verifiable fence evidence.
        FenceOutcome::Fenced
    }

    async fn fence_unregistered(&self, claim: &StateClaim) -> FenceOutcome {
        // launch creates this exact group before spawning. Its absence means
        // nothing was spawned on this boot; a process left in the daemon's own
        // group never received an envelope and exits on handoff EOF.
        let path = self.root.join(format!("shaula-{}", claim.worker_attempt));
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return FenceOutcome::Fenced
            }
            Ok(meta) if meta.is_dir() => {}
            _ => return FenceOutcome::Unknown,
        }
        if !containment::kill(&path).await {
            return FenceOutcome::Unknown;
        }
        // No durable identity names this group, so it is not restart evidence.
        // A leftover empty group is harmless; the replacement uses a new attempt.
        let _ = std::fs::remove_dir(&path);
        FenceOutcome::Fenced
    }
}
