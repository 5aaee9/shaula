//! Runner generation lifecycle states and transitions (spec 0001 §10.1).

use serde::{Deserialize, Serialize};

/// Coarse externally visible lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationState {
    CreatePending,
    Creating,
    WaitingOnline,
    Idle,
    Busy,
    Retiring,
    DestroyPending,
    Destroying,
    CleanupRequired,
    Quarantined,
    Destroyed,
}

impl GenerationState {
    /// Canonical string representation persisted in the ledger.
    pub fn as_str_repr(self) -> &'static str {
        match self {
            GenerationState::CreatePending => "CreatePending",
            GenerationState::Creating => "Creating",
            GenerationState::WaitingOnline => "WaitingOnline",
            GenerationState::Idle => "Idle",
            GenerationState::Busy => "Busy",
            GenerationState::Retiring => "Retiring",
            GenerationState::DestroyPending => "DestroyPending",
            GenerationState::Destroying => "Destroying",
            GenerationState::CleanupRequired => "CleanupRequired",
            GenerationState::Quarantined => "Quarantined",
            GenerationState::Destroyed => "Destroyed",
        }
    }

    /// Inverse of [`GenerationState::as_str_repr`]; unknown strings are
    /// corrupt ledger data and must fail closed.
    pub fn from_str_repr(value: &str) -> Result<Self, String> {
        match value {
            "CreatePending" => Ok(GenerationState::CreatePending),
            "Creating" => Ok(GenerationState::Creating),
            "WaitingOnline" => Ok(GenerationState::WaitingOnline),
            "Idle" => Ok(GenerationState::Idle),
            "Busy" => Ok(GenerationState::Busy),
            "Retiring" => Ok(GenerationState::Retiring),
            "DestroyPending" => Ok(GenerationState::DestroyPending),
            "Destroying" => Ok(GenerationState::Destroying),
            "CleanupRequired" => Ok(GenerationState::CleanupRequired),
            "Quarantined" => Ok(GenerationState::Quarantined),
            "Destroyed" => Ok(GenerationState::Destroyed),
            other => Err(format!("unknown generation state {other:?}")),
        }
    }
    /// Whether the generation counts toward Effective Capacity.
    pub fn counts_effective(self) -> bool {
        matches!(
            self,
            GenerationState::CreatePending
                | GenerationState::Creating
                | GenerationState::WaitingOnline
                | GenerationState::Idle
                | GenerationState::Busy
        )
    }

    /// Whether the generation counts toward Resource Occupancy: every
    /// generation whose Destroy has not completed, including cleanup,
    /// retiring and quarantined states.
    pub fn counts_occupancy(self) -> bool {
        self != GenerationState::Destroyed
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, GenerationState::Destroyed)
    }

    pub fn is_quarantined(self) -> bool {
        self == GenerationState::Quarantined
    }
}

/// Allowed coarse transitions. Anything not listed here is a bug and must be
/// rejected rather than silently applied.
pub fn transition_allowed(from: GenerationState, to: GenerationState) -> bool {
    use GenerationState::*;
    match (from, to) {
        (CreatePending, Creating) => true,
        (Creating, WaitingOnline) => true,
        (Creating, CleanupRequired) => true,
        (WaitingOnline, Idle) => true,
        (WaitingOnline, CleanupRequired) => true,
        (Idle, Busy) => true,
        (Busy, Retiring) => true,
        (Idle, Retiring) => true,

        (CleanupRequired, Retiring) => true,
        (CleanupRequired, Quarantined) => true,
        (Retiring, Retiring) => true,
        (Retiring, DestroyPending) => true,
        (DestroyPending, Destroying) => true,
        (Destroying, Destroyed) => true,
        (Destroying, DestroyPending) => true,
        // Quarantine is reachable from every non-terminal state.
        (CreatePending, Quarantined)
        | (Creating, Quarantined)
        | (WaitingOnline, Quarantined)
        | (Idle, Quarantined)
        | (Busy, Quarantined)
        | (Retiring, Quarantined)
        | (DestroyPending, Quarantined)
        | (Destroying, Quarantined) => true,
        _ => false,
    }
}

/// Attempt to advance a state; returns the error when the transition is not
/// part of the contract diagram.
pub fn advance(current: GenerationState, next: GenerationState) -> Result<GenerationState, String> {
    if transition_allowed(current, next) {
        Ok(next)
    } else {
        Err(format!(
            "illegal lifecycle transition {current:?} -> {next:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use GenerationState::*;

    #[test]
    fn happy_path_create_to_destroyed() {
        let path = [
            (CreatePending, Creating),
            (Creating, WaitingOnline),
            (WaitingOnline, Idle),
            (Idle, Retiring),
            (Retiring, DestroyPending),
            (DestroyPending, Destroying),
            (Destroying, Destroyed),
        ];
        for (from, to) in path {
            assert!(
                transition_allowed(from, to),
                "{from:?} -> {to:?} must be allowed"
            );
        }
    }

    #[test]
    fn uncertain_create_enters_cleanup_not_reapply() {
        assert!(transition_allowed(Creating, CleanupRequired));
        assert!(transition_allowed(CleanupRequired, Retiring));
        assert!(!transition_allowed(CleanupRequired, Creating));
        assert!(!transition_allowed(CleanupRequired, WaitingOnline));
    }

    #[test]
    fn retiring_self_loop_for_job_still_running() {
        assert!(transition_allowed(Retiring, Retiring));
    }

    #[test]
    fn quarantine_reachable_everywhere_nonterminal() {
        for state in [
            CreatePending,
            Creating,
            WaitingOnline,
            Idle,
            Busy,
            Retiring,
            DestroyPending,
            Destroying,
        ] {
            assert!(
                transition_allowed(state, Quarantined),
                "{state:?} -> Quarantined"
            );
        }
        assert!(!transition_allowed(Destroyed, Quarantined));
    }

    #[test]
    fn busy_does_not_go_directly_to_destroy() {
        assert!(!transition_allowed(Busy, DestroyPending));
        assert!(transition_allowed(Busy, Retiring));
    }

    #[test]
    fn occupancy_and_effective_partition() {
        // Every occupied state except the five effective ones (cleanup,
        // retiring, destroy-pending, destroying, quarantined) still holds
        // capacity slots.
        assert!(CreatePending.counts_effective() && CreatePending.counts_occupancy());
        assert!(Busy.counts_effective() && Busy.counts_occupancy());
        assert!(!CleanupRequired.counts_effective() && CleanupRequired.counts_occupancy());
        assert!(!Quarantined.counts_effective() && Quarantined.counts_occupancy());
        assert!(!Destroyed.counts_effective() && !Destroyed.counts_occupancy());
    }

    #[test]
    fn illegal_backward_transition_rejected() {
        assert!(advance(Destroyed, Idle).is_err());
        assert!(advance(Idle, Creating).is_err());
        assert!(
            advance(Quarantined, Idle).is_err(),
            "quarantine requires explicit operator procedure"
        );
    }
}
