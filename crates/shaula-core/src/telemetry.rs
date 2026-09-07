//! Telemetry port: bounded metric emission with a finite attribute
//! allowlist. High-cardinality identifiers (fleet keys, revisions, digests,
//! actors, targets, runner/job/workspace identities, URLs, error text) are
//! structurally excluded.

use std::sync::Arc;

/// Bounded operation kinds allowed as metric attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricOperation {
    Http,
    RegistryAdmission,
    ChangeReconcile,
    AuthRollout,
    SessionEstablish,
    MessageIngest,
    Inventory,
    Reaper,
    RunnerCreate,
    RunnerDestroy,
    IacOperation,
    ArtifactPublish,
    ProfileValidate,
}

impl MetricOperation {
    pub const ALL: &'static [MetricOperation] = &[
        MetricOperation::Http,
        MetricOperation::RegistryAdmission,
        MetricOperation::ChangeReconcile,
        MetricOperation::AuthRollout,
        MetricOperation::SessionEstablish,
        MetricOperation::MessageIngest,
        MetricOperation::Inventory,
        MetricOperation::Reaper,
        MetricOperation::RunnerCreate,
        MetricOperation::RunnerDestroy,
        MetricOperation::IacOperation,
        MetricOperation::ArtifactPublish,
        MetricOperation::ProfileValidate,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MetricOperation::Http => "http",
            MetricOperation::RegistryAdmission => "registry_admission",
            MetricOperation::ChangeReconcile => "change_reconcile",
            MetricOperation::AuthRollout => "auth_rollout",
            MetricOperation::SessionEstablish => "session_establish",
            MetricOperation::MessageIngest => "message_ingest",
            MetricOperation::Inventory => "inventory",
            MetricOperation::Reaper => "reaper",
            MetricOperation::RunnerCreate => "runner_create",
            MetricOperation::RunnerDestroy => "runner_destroy",
            MetricOperation::IacOperation => "iac_operation",
            MetricOperation::ArtifactPublish => "artifact_publish",
            MetricOperation::ProfileValidate => "profile_validate",
        }
    }
}

/// Bounded result values allowed as metric attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricResult {
    Ok,
    Failed,
    Uncertain,
    Blocked,
    RateLimited,
}

impl MetricResult {
    pub fn as_str(self) -> &'static str {
        match self {
            MetricResult::Ok => "ok",
            MetricResult::Failed => "failed",
            MetricResult::Uncertain => "uncertain",
            MetricResult::Blocked => "blocked",
            MetricResult::RateLimited => "rate_limited",
        }
    }
}

/// Bounded telemetry attribute set. Construction is exhaustive over finite
/// enums only, so no unbounded value can become a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MetricAttributes {
    pub operation: MetricOperation,
    pub result: MetricResult,
}

/// Monotonic counter with bounded series. Each distinct attribute tuple is
/// one series; the allowlist bounds the total number of series. All access
/// is serialized by the owning `Mutex` in `TelemetryHandle` — a plain u64
/// is enough; no second synchronization mechanism.
#[derive(Debug, Default)]
pub struct CounterRegistry {
    counters: Vec<(MetricAttributes, u64)>,
}

impl CounterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn increment(&mut self, attributes: MetricAttributes, delta: u64) {
        let entry = self
            .counters
            .iter_mut()
            .find(|(attrs, _)| *attrs == attributes)
            .map(|(_, counter)| counter);
        match entry {
            Some(counter) => *counter = counter.wrapping_add(delta),
            None => self.counters.push((attributes, delta)),
        }
    }

    /// Snapshot in bounded series order.
    pub fn snapshot(&self) -> Vec<(MetricAttributes, u64)> {
        self.counters.clone()
    }
}

/// Shared telemetry sink. Production: tracing + OpenTelemetry pipeline.
/// Tests: in-memory adapter observing spans and measurements.
#[derive(Debug, Clone)]
pub struct TelemetryHandle {
    registry: Arc<std::sync::Mutex<CounterRegistry>>,
}

impl Default for TelemetryHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryHandle {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(std::sync::Mutex::new(CounterRegistry::new())),
        }
    }

    /// Emits a bounded counter increment and a tracing event with only
    /// bounded attributes.
    pub fn record(&self, attributes: MetricAttributes, delta: u64) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.increment(attributes, delta);
        }
    }

    pub fn snapshot(&self) -> Vec<(MetricAttributes, u64)> {
        self.registry
            .lock()
            .map(|r| r.snapshot())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_are_bounded_by_allowlist() {
        let mut registry = CounterRegistry::new();
        for operation in MetricOperation::ALL {
            registry.increment(
                MetricAttributes {
                    operation: *operation,
                    result: MetricResult::Ok,
                },
                1,
            );
        }
        let snapshot = registry.snapshot();
        assert_eq!(
            snapshot.len(),
            MetricOperation::ALL.len(),
            "one series per operation at a fixed result"
        );
    }

    #[test]
    fn same_attributes_aggregate_into_one_series() {
        let mut registry = CounterRegistry::new();
        let attrs = MetricAttributes {
            operation: MetricOperation::RunnerCreate,
            result: MetricResult::Ok,
        };
        registry.increment(attrs, 2);
        registry.increment(attrs, 3);
        assert_eq!(registry.snapshot(), vec![(attrs, 5)]);
    }

    #[test]
    fn telemetry_handle_is_shareable() {
        let handle = TelemetryHandle::new();
        let clone = handle.clone();
        handle.record(
            MetricAttributes {
                operation: MetricOperation::Http,
                result: MetricResult::Failed,
            },
            1,
        );
        clone.record(
            MetricAttributes {
                operation: MetricOperation::Http,
                result: MetricResult::Ok,
            },
            2,
        );
        assert_eq!(handle.snapshot().len(), 2);
    }
}
