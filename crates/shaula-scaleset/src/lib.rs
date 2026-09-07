//! Pure Rust GitHub/Actions Service Scale Set adapter.
//!
//! Implements the protocol of the pinned `github.com/actions/scaleset`
//! commit `cb0405b2d874500e75ae34eff8d582ab75956b45` with reqwest. The Go
//! SDK and its `internal/testserver` are differential-test oracles only;
//! they are neither linked nor shipped.

pub mod auth;
pub mod client;
pub mod config;
pub mod error;
pub mod port;
pub mod wire;

pub use auth::Credential;
pub use client::ScalesetClient;
pub use error::ScalesetError;

/// The pinned upstream oracle commit this adapter is written against.
/// Upgrading requires reviewing the Go source diff and re-running the
/// differential suites.
pub const ORACLE_COMMIT: &str = "cb0405b2d874500e75ae34eff8d582ab75956b45";
