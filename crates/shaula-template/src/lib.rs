//! Artifact/workspace management and the Terraform subprocess runtime.

pub mod artifact;
pub mod engine;
pub mod http_backend;
pub mod manifest;
pub mod runtime;
#[doc(hidden)]
pub mod ssh_helper;
pub mod variables;
pub mod workspace;
pub mod workspace_materialize;

pub use artifact::ArtifactStore;
pub mod artifact_cache;
pub use runtime::TemplateRuntime;
mod artifact_integrity;
mod docker_connection;
mod operation_capture;
mod operation_sanitize;

#[cfg(test)]
mod aws_tests;
#[cfg(test)]
mod cloud_tests;
#[cfg(test)]
mod proxmox_tests;
