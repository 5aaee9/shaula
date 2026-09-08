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
    artifact_cache:
        Option<std::sync::Arc<dyn shaula_core::registry::template_library::ArtifactCache>>,
    auth_observations: auth_observations::RouteObservations,
}

impl SqliteControlPlane {
    pub fn new(store: Store, artifact_root: PathBuf) -> Self {
        Self {
            store,
            artifact_root,
            artifact_cache: None,
            auth_observations: auth_observations::RouteObservations::default(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn with_artifact_cache(
        mut self,
        cache: std::sync::Arc<dyn shaula_core::registry::template_library::ArtifactCache>,
    ) -> Self {
        self.artifact_cache = Some(cache);
        self
    }

    async fn artifact_available(&self, digest: &str) -> shaula_core::error::CoreResult<bool> {
        match &self.artifact_cache {
            Some(cache) => cache.ensure_cached(digest).await,
            None => Ok(true),
        }
    }
}

mod artifact_reads;
#[path = "artifacts.rs"]
mod artifacts;

mod auth_execution;
#[path = "control_plane_store.rs"]
mod control_plane_store;

#[path = "commits.rs"]
mod commits;

#[path = "commits_fleet.rs"]
mod commits_fleet;
mod commits_fleet_noop;
mod commits_noop;
mod retirement;

#[path = "mapping.rs"]
mod mapping;

#[path = "lifecycle_impl.rs"]
mod lifecycle_impl;
mod lifecycle_support;

#[path = "scan.rs"]
mod scan;
mod template_scan;

pub use scan::ScanReport;

mod auth_observations;
