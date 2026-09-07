//! Implementation of the core `ControlPlaneStore` port over SQLite plus the
//! artifact directory. Every composite mutation commits revision, change,
//! audit, outbox and idempotency facts in one short transaction.

use std::path::PathBuf;

use shaula_core::error::{CoreError, ReasonCode};

use crate::store::Store;

pub(crate) fn core_err(e: crate::store::StoreError) -> CoreError {
    CoreError::new(ReasonCode::StorageUnavailable, e.to_string())
}

/// The concrete store facade handed to the daemon. `artifact_root` serves
/// digest-addressed manifest/shape lookups without importing the template
/// runtime.
pub struct SqliteControlPlane {
    store: Store,
    artifact_root: PathBuf,
}

impl SqliteControlPlane {
    pub fn new(store: Store, artifact_root: PathBuf) -> Self {
        Self {
            store,
            artifact_root,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }
}

#[path = "artifacts.rs"]
mod artifacts;

#[path = "control_plane_store.rs"]
mod control_plane_store;

#[path = "commits.rs"]
mod commits;

#[path = "commits_fleet.rs"]
mod commits_fleet;
mod commits_noop;
mod retirement;

#[path = "mapping.rs"]
mod mapping;

#[path = "lifecycle_impl.rs"]
mod lifecycle_impl;
mod lifecycle_support;

#[path = "scan.rs"]
mod scan;

pub use scan::ScanReport;
