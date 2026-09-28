//! Internal lifecycle authority. These types carry no management credentials.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::secret::SecretString;
use crate::state_backend::{StateCapability, StateClaim, StateError, StateResult};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_CONTROL_BYTES: usize = 64 * 1024;
pub const MAX_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(86_400);

pub mod cleanup;
pub mod journal;
pub mod wire;

/// Control and Terraform state credentials are deliberately different types
/// and verifier domains. Neither grants a management API scope.
#[derive(Debug, Clone)]
pub struct ControlCapability(SecretString);

impl ControlCapability {
    pub fn issue() -> Self {
        Self(SecretString::new(format!(
            "sc1_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        )))
    }

    pub fn parse(value: SecretString) -> StateResult<Self> {
        let random = value
            .expose()
            .strip_prefix("sc1_")
            .ok_or(StateError::Unauthorized)?;
        if random.len() != 64 || !random.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(StateError::Unauthorized);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }

    pub fn verifier(&self) -> Vec<u8> {
        let mut hash = Sha256::new();
        hash.update(b"shaula-control-capability-v1\0");
        hash.update(self.expose().as_bytes());
        hash.finalize().to_vec()
    }

    pub fn matches(&self, verifier: &[u8]) -> bool {
        bool::from(self.verifier().ct_eq(verifier))
    }
}

/// Issued before admission; only hashes are persisted. A failed admission
/// discards this bundle. A lost handoff requires recovery, never token reuse
/// with a different Claim.
#[derive(Debug, Clone)]
pub struct WorkerAdmission {
    pub claim: StateClaim,
    pub control: ControlCapability,
    pub state: StateCapability,
    pub cleanup_only: bool,
}

impl WorkerAdmission {
    pub fn new(generation_id: Uuid) -> Self {
        Self {
            claim: StateClaim {
                generation_id,
                worker_epoch: 1,
                worker_attempt: Uuid::new_v4(),
            },
            control: ControlCapability::issue(),
            state: StateCapability::issue(),
            cleanup_only: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ControlAccess {
    pub claim: StateClaim,
    pub capability: ControlCapability,
}

/// Observations are not fencing receipts. Unknown never permits replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessObservation {
    Live,
    Exited,
    Unknown,
}

/// Identity captured by the executor before admitting any external effect.
/// A PID alone is insufficient across restart or reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub host_boot: String,
    pub process_id: u32,
    pub started: String,
    pub containment: String,
    /// Kernel ID of the delegated root enclosing `containment`. The kernel
    /// removes a cgroup only when its whole subtree is empty, so a different
    /// root ID on the same boot proves the old tree stopped. Absent on
    /// identities recorded before it existed; those never use this proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<u64>,
}

impl ProcessIdentity {
    pub fn validate(&self) -> StateResult<()> {
        if self.process_id == 0
            || [&self.host_boot, &self.started, &self.containment]
                .iter()
                .any(|value| {
                    value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
                })
        {
            return Err(StateError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    ProviderCleanup,
    NeverStarted,
    OperatorAttested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionReceipt {
    pub claim: StateClaim,
    pub request_id: Uuid,
    pub state_revision: i64,
    pub kind: CompletionKind,
    pub completed_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceOutcome {
    Fenced,
    Unknown,
}

#[async_trait::async_trait]
pub trait Executor: Send + Sync {
    /// Starts only the exact job executable, blocked on its protected handoff.
    async fn launch(&self, claim: &StateClaim) -> StateResult<ProcessIdentity>;
    /// Called after the daemon durably registers the executor's identity.
    async fn handoff(&self, envelope: wire::LaunchEnvelope) -> StateResult<()>;
    async fn observe(&self, identity: &ProcessIdentity) -> ProcessObservation;
    async fn stop_and_fence(&self, identity: &ProcessIdentity) -> FenceOutcome;
    /// Restart-only fence for a launch_pending attempt whose identity never
    /// became durable. Handoff follows registration, so no envelope was sent;
    /// any process must still be proved absent from its exact containment.
    async fn fence_unregistered(&self, claim: &StateClaim) -> FenceOutcome;
    /// Only after a durable fenced or cleaned terminal record replaces the
    /// kernel directory as restart evidence. Never deletes a populated group.
    async fn release_fenced(&self, identity: &ProcessIdentity) -> StateResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_separate_and_redacted() {
        let admission = WorkerAdmission::new(Uuid::new_v4());
        assert!(admission.control.matches(&admission.control.verifier()));
        assert!(!admission.control.matches(&admission.state.verifier()));
        assert!(ControlCapability::parse(SecretString::new(admission.state.expose())).is_err());
        assert!(StateCapability::parse(SecretString::new(admission.control.expose())).is_err());
        let debug = format!("{admission:?}");
        assert!(!debug.contains(admission.control.expose()));
        assert!(!debug.contains(admission.state.expose()));
    }

    #[test]
    fn identities_recorded_before_root_ids_still_parse_unchanged() -> StateResult<()> {
        let legacy =
            r#"{"host_boot":"m:b","process_id":7,"started":"42","containment":"/c/shaula-x"}"#;
        let identity: ProcessIdentity =
            serde_json::from_str(legacy).map_err(|_| StateError::Invalid)?;
        assert_eq!(identity.root_id, None);
        assert_eq!(
            serde_json::to_string(&identity).map_err(|_| StateError::Invalid)?,
            legacy
        );
        let current: ProcessIdentity = serde_json::from_str(
            r#"{"host_boot":"m:b","process_id":7,"started":"42","containment":"/c/shaula-x","root_id":9}"#,
        )
        .map_err(|_| StateError::Invalid)?;
        assert_eq!(current.root_id, Some(9));
        Ok(())
    }
}
