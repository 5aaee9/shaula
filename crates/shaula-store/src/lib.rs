//! The sole SeaORM boundary: connection management, SQLite pragmas and
//! typed repositories. No `DatabaseConnection`, entity or ORM error escapes
//! this crate.

pub mod auth_repo;
mod entities;
pub mod fleet_auth_repo;
pub mod fleet_repo;
pub mod lifecycle_repo;
pub mod registry_impl;
pub mod scaleset_ownership_repo;
pub mod shared_repo;
pub mod store;
pub mod template_repo;

pub use store::{Store, StoreError, StoreResult};

#[cfg(test)]
mod tests {
    mod apply_fence;
    mod auth_rotation;
    mod persistence;
    mod profiles;
    mod profiles_attestation;
}
