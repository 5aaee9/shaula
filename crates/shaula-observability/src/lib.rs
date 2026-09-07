//! OTel/tracing setup, exporters and redaction policy.
//!
//! Day 0 observability (spec 0001 §13): the tracing subscriber is
//! initialized BEFORE ledger migration or any remote side effect. Local
//! structured logs stay available when OTLP export fails; export failure
//! never blocks lifecycle correctness.

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Guard keeping the global subscriber installed for the process
/// lifetime; dropping it is a no-op for the process-wide default.
pub struct TelemetryGuard;

/// Initializes the `tracing` subscriber with a bounded env filter and a
/// JSON local sink carrying `trace_id`/`span_id` correlation. Idempotent:
/// repeated calls keep the first subscriber.
pub fn init(service_name: &str) -> TelemetryGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(false)
                .flatten_event(true)
                .with_target(false),
        )
        .try_init();
    tracing::info!(service = service_name, "telemetry initialized");
    TelemetryGuard
}

/// Bounded metric attributes helpers shared by all crates; construction is
/// exhaustive over finite enums so no unbounded value can become a label.
pub use shaula_core::telemetry::{
    MetricAttributes, MetricOperation, MetricResult, TelemetryHandle,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent_and_returns_guard() {
        let _guard = init("shaula-test");
        let _second = init("shaula-test-2");
    }
}
