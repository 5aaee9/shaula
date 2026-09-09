//! Captured authority for session installation and Fleet observations.

use crate::auth_context::ResolvedAuthContext;
use crate::error::ReasonCode;
use crate::ports::SessionHandle;

/// The exact desired Fleet head captured before asynchronous work begins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetRuntimeGuard {
    pub incarnation: String,
    pub desired_revision: i64,
    pub mutation_fence: i64,
}

impl From<&super::FleetHead> for FleetRuntimeGuard {
    fn from(head: &super::FleetHead) -> Self {
        Self {
            incarnation: head.incarnation.clone(),
            desired_revision: head.desired_revision,
            mutation_fence: head.mutation_fence,
        }
    }
}

/// A remotely established session, committed only if its captured authority
/// and predecessor epoch still match. Queue credentials remain Debug-redacted.
#[derive(Debug, Clone)]
pub struct SessionInstall {
    pub guard: FleetRuntimeGuard,
    pub expected_epoch: Option<i64>,
    pub auth_context: ResolvedAuthContext,
    pub scale_set_id: i64,
    pub handle: SessionHandle,
}

/// The complete durable handle of an active session.
#[derive(Debug, Clone)]
pub struct PersistedSession {
    pub epoch: i64,
    pub last_message_id: i64,
    pub scale_set_id: i64,
    pub auth_context: ResolvedAuthContext,
    pub handle: SessionHandle,
}

/// Runtime observations cannot write deletion or admission phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetObservationPhase {
    Reconciling,
    Ready,
    Degraded,
}

impl FleetObservationPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reconciling => "Reconciling",
            Self::Ready => "Ready",
            Self::Degraded => "Degraded",
        }
    }
}

/// One classified Fleet head and its corresponding Change transition.
#[derive(Debug, Clone)]
pub struct FleetObservation {
    pub guard: FleetRuntimeGuard,
    /// Active session epoch captured by the observer; `None` requires no
    /// active session. Cleared epochs never establish listener readiness.
    pub session_epoch: Option<i64>,
    pub phase: FleetObservationPhase,
    pub reason: Option<ReasonCode>,
}
