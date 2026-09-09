//! Shaula core: domain model, invariants, state machines and ports.
//!
//! This crate stays deterministic under injected clock/ID generators and
//! fake ports. Axum request types, SeaORM models and Actions Service JSON
//! types are mapped at adapter boundaries, never here.

pub mod artifact_layout;
pub mod auth;
pub mod auth_context;
pub mod auth_policy;
pub mod capacity;
pub mod error;
pub mod fleet;
pub mod github;
pub mod jobs;
pub mod lifecycle;
pub mod lockfile;
pub mod net;
pub mod operation_log;
pub mod plan;
pub mod ports;
pub mod registry;
pub mod secret;
pub mod setup_info;
pub mod state_backend;
pub mod telemetry;
pub mod template;

pub use error::CoreError;
pub use error::CoreResult;
pub use error::ReasonCode;
