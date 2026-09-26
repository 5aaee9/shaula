//! Conditional publication contracts through the HTTP and Registry interfaces.

// Existing shared fixtures intentionally panic on setup failures; keep the
// exception scoped to them, not to the new publication contract tests.
#[allow(clippy::unwrap_used)]
#[path = "../common/mod.rs"]
mod common;
mod concurrency;
mod conditions;
mod history;
mod pool;
mod read_failure;
mod registry;
mod support;
mod transactions;
