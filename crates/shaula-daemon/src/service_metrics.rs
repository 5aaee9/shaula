//! Registry admission metrics: only the finite operation/result labels of
//! spec 0001 §13.2 — never resource identity, error text or actor names.

use shaula_core::error::CoreResult;
use shaula_core::registry::MutationError;
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

/// Records one registry mutation outcome: an accepted commit is `ok`, a
/// rejected or precondition-failed mutation is `degraded`, and a store or
/// transport fault is `failed`.
pub(crate) fn record_admission<T>(outcome: &CoreResult<Result<T, MutationError>>) {
    let result = match outcome {
        Ok(Ok(_)) => MetricResult::Ok,
        Ok(Err(_)) => MetricResult::Degraded,
        Err(_) => MetricResult::Failed,
    };
    TelemetryHandle::new().record(MetricOperation::Registry, result, 1);
}
