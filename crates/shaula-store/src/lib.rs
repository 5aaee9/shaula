//! The sole SeaORM boundary: connection management, SQLite pragmas and
//! typed repositories. No `DatabaseConnection`, entity or ORM error escapes
//! this crate.

pub mod artifact_library;
mod auth_dependents_repo;
mod auth_execution_repo;
pub mod auth_policy_repo;
pub mod auth_repo;
pub mod auth_v2_repo;
mod entities;
mod fleet_auth_ack;
pub mod fleet_auth_context_repo;
pub mod fleet_auth_repo;
mod fleet_observations;
pub mod fleet_repo;
pub mod http_state;
pub mod jobs;
pub mod lifecycle_repo;
mod listener_acquisitions;
mod listener_messages;
mod listener_observations;
pub mod operation_logs;
pub mod registry_impl;
mod runtime_guards;
pub mod scaleset_ownership_repo;
pub mod setup_info;
pub mod shared_repo;
pub mod store;
mod template_activation;
pub mod template_repo;

pub use store::{Store, StoreError, StoreResult};

#[cfg(test)]
mod tests {
    mod apply_fence;
    mod auth_concurrency;
    mod auth_execution;
    mod auth_execution_guards;
    mod auth_fixture;
    mod auth_handoff_failures;
    mod auth_rotation;
    mod auth_v2;
    mod auth_v2_context;
    mod auth_validation_results;
    mod fleet_convergence;
    mod listener_lifecycle;
    pub(crate) mod listener_messages;
    mod listener_recovery;
    mod migration_format;
    mod persistence;
    mod profiles;
    mod profiles_attestation;
    mod runtime_observations;
    mod runtime_sessions;
    mod session_support;
    mod template_activation;
    mod template_activation_support;
    mod template_scan;
    mod write_contention;
}
