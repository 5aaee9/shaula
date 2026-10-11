//! Short-circuit plumbing for the v2 validation stages: every stage either
//! proves its facts or stops the pass with a terminal rejection reason, a
//! bounded retry, or a hard error. The caller maps the stop onto exactly one
//! durable Candidate outcome.

use shaula_core::auth_validation::ProbeOutcome;
use shaula_core::error::CoreError;

/// Why a validation pass stopped before promotion.
pub(super) enum Stop {
    /// Terminal: durably reject the Candidate with this reason.
    Rejected(&'static str),
    /// Transient: keep the Candidate Validating with a bounded retry.
    Retry { retry_after_ms: Option<i64> },
    /// Hard failure propagated to the caller unchanged.
    Failed(CoreError),
}

impl From<CoreError> for Stop {
    fn from(error: CoreError) -> Self {
        Self::Failed(error)
    }
}

/// A validation stage result.
pub(super) type Step<T> = Result<T, Stop>;

/// Maps a probe outcome onto a stage result.
pub(super) fn probe(outcome: ProbeOutcome) -> Step<()> {
    match outcome {
        ProbeOutcome::Proven => Ok(()),
        ProbeOutcome::Terminal(reason) => Err(Stop::Rejected(reason)),
        ProbeOutcome::Retry { retry_after_ms } => Err(Stop::Retry { retry_after_ms }),
    }
}
