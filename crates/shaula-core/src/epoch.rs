//! Session epoch semantics (spec 0001 §8): monotonic per-fleet generation
//! counter whose durable CAS gates every outbound effect of a poll task.

use serde::{Deserialize, Serialize};

/// Fleet-scoped, monotonically increasing persisted epoch. Session
/// install/replace advances it only after old-epoch outbound effects are
/// drained or durably classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionEpoch(pub u64);

impl SessionEpoch {
    pub fn initial() -> Self {
        Self(1)
    }

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// The verdict of a durable compare-and-set against the current epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochCas {
    /// The task's epoch is current; it may ACK/acquire/write demand.
    Current,
    /// The task captured a stale epoch and must no-op: it may not ACK,
    /// acquire, overwrite demand or wake lifecycle.
    Stale,
}

/// Pure decision helper: whether a task holding `captured` may proceed given
/// the currently persisted `current` epoch.
pub fn authorize(captured: SessionEpoch, current: SessionEpoch) -> EpochCas {
    if captured == current {
        EpochCas::Current
    } else {
        EpochCas::Stale
    }
}

/// Poll task lifecycle hooks used by the daemon listener supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundEffect {
    Ack { message_id: i64 },
    Acquire { runner_request_id: i64 },
    DemandWrite,
    LifecycleWake,
}

impl OutboundEffect {
    /// Whether the stale-epoch task may perform this effect. Per contract
    /// only no-ops are allowed for every effect class.
    pub fn allowed_when_stale(self) -> bool {
        let _ = self;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epochs_are_monotonic() {
        let e0 = SessionEpoch::initial();
        let e1 = e0.next();
        assert!(e1 > e0);
        assert_ne!(e0, e1);
    }

    #[test]
    fn stale_task_cannot_act() {
        let current = SessionEpoch(7);
        let stale = SessionEpoch(6);
        assert_eq!(authorize(stale, current), EpochCas::Stale);
        assert!(!OutboundEffect::Ack { message_id: 1 }.allowed_when_stale());
        assert!(!OutboundEffect::Acquire {
            runner_request_id: 2
        }
        .allowed_when_stale());
        assert!(!OutboundEffect::DemandWrite.allowed_when_stale());
        assert!(!OutboundEffect::LifecycleWake.allowed_when_stale());
    }

    #[test]
    fn current_epoch_authorizes() {
        let current = SessionEpoch(7);
        assert_eq!(authorize(current, current), EpochCas::Current);
    }
}
