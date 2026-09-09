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
        let reason = match failure {
            AccessFailure::Unauthenticated | AccessFailure::SessionExpired => {
                ReasonCode::Unauthenticated
            }
            AccessFailure::PermissionDenied => ReasonCode::PermissionDenied,
            AccessFailure::TargetHiddenOrNotFound => ReasonCode::TargetHiddenOrNotFound,
            AccessFailure::RateLimited { .. } => ReasonCode::RateLimited,
            AccessFailure::Unavailable { .. } | AccessFailure::RequestUncertain { .. } => {
                ReasonCode::AccessVerificationFailed
            }
        };
        Self::Blocked(reason)
    }
}
