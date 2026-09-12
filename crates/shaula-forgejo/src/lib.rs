//! Forgejo Actions runner control-plane adapter.
//!
//! This crate deliberately does not implement the GitHub scale-set port. Forgejo
//! runners have different ownership and job-association semantics, so the
//! adapter exposes only the provider-specific pool contract.

mod body;
mod client;
mod error;
mod labels;
mod models;
mod port;
mod uncertain;

pub use client::ForgejoClient;
pub use error::ForgejoError;
pub use models::{
    ForgejoScope, Job, Label, Registration, RegistrationUncertainty, Removal, Runner,
    RunnerBootstrapMaterial, RunnerStatus,
};

#[cfg(test)]
mod http_safety_tests;
#[cfg(test)]
mod tests;
