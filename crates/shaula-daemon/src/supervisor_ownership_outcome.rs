//! Finite ownership diagnostics for Fleet conditions; provider text stays out.

use shaula_core::error::ReasonCode;
use shaula_core::ports::AccessFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipOutcome {
    Ready,
    Blocked(ReasonCode),
}

impl OwnershipOutcome {
    pub(super) fn access_failure(failure: &AccessFailure) -> Self {
        Self::Blocked(crate::listener::access_reason(failure))
    }
}
