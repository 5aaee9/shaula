//! Pure Rust GitHub/Actions Service Scale Set adapter.
//!
//! Implements the protocol of the pinned `github.com/actions/scaleset`
//! commit `cb0405b2d874500e75ae34eff8d582ab75956b45` with reqwest. The Go
//! SDK and its `internal/testserver` are differential-test oracles only;
//! they are neither linked nor shipped.

mod app_installation_link;
pub mod auth;
pub mod client;
pub mod config;
pub mod error;
pub mod installation;
pub mod installation_tokens;
pub mod port;
mod route_identity;
pub mod route_proof;
pub mod wire;

pub use auth::Credential;
pub use client::ScalesetClient;
pub use error::ScalesetError;
pub use installation::{AppInstallationResolver, InstallationLookup};

/// The pinned upstream oracle commit this adapter is written against.
/// Upgrading requires reviewing the Go source diff and re-running the
/// differential suites.
pub const ORACLE_COMMIT: &str = "cb0405b2d874500e75ae34eff8d582ab75956b45";
