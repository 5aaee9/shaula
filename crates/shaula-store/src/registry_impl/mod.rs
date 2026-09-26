//! Implementation of the core `ControlPlaneStore` port over SQLite plus the
//! artifact directory. Every composite mutation commits revision, change,
//! audit, outbox and idempotency facts in one short transaction.

use std::path::PathBuf;

use shaula_core::error::{CoreError, ReasonCode};

use crate::store::Store;

pub(crate) fn core_err(e: crate::store::StoreError) -> CoreError {
    match e {
        crate::store::StoreError::Corrupt(summary) => {
            CoreError::new(ReasonCode::StorageCorrupt, summary)
        }
        error => CoreError::new(ReasonCode::StorageUnavailable, error.to_string()),
    }
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
    worker_admissions: Option<std::sync::Arc<crate::http_state::WorkerAdmissions>>,
}

impl SqliteControlPlane {
    pub fn new(store: Store, artifact_root: PathBuf) -> Self {
        Self {
            store,
            artifact_root,
            artifact_cache: None,
            auth_observations: auth_observations::RouteObservations::default(),
            worker_admissions: None,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn with_worker_admissions(
        mut self,
        admissions: std::sync::Arc<crate::http_state::WorkerAdmissions>,
    ) -> Self {
        self.worker_admissions = Some(admissions);
        self
    }

    async fn admit_worker_on(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        id: &str,
    ) -> shaula_core::error::CoreResult<Option<shaula_core::worker::WorkerAdmission>> {
        match &self.worker_admissions {
            Some(admissions) => admissions
                .admit_on(tx, id)
                .await
                .map(Some)
                .map_err(worker_error),
            None => Ok(None),
        }
    }

    fn publish_worker(
        &self,
        admission: Option<shaula_core::worker::WorkerAdmission>,
    ) -> shaula_core::error::CoreResult<()> {
        if let (Some(admissions), Some(admission)) = (&self.worker_admissions, admission) {
            admissions.publish(admission).map_err(worker_error)?;
        }
        Ok(())
    }

    async fn complete_worker_generation(
        &self,
        id: &str,
        now: i64,
    ) -> shaula_core::error::CoreResult<bool> {
        let completed = crate::http_state::SqliteStateBackend::new(self.store.clone())
            .complete_generation(id, now)
            .await
            .map_err(worker_error)?;
        if completed {
            if let Some(admissions) = &self.worker_admissions {
                admissions.discard(id);
            }
        }
        Ok(completed)
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

fn worker_error(_: shaula_core::state_backend::StateError) -> CoreError {
    CoreError::new(
        ReasonCode::StorageUnavailable,
        "lifecycle worker admission unavailable",
    )
}

mod artifact_reads;
#[path = "artifacts.rs"]
mod artifacts;

mod auth_execution;
#[path = "control_plane_store.rs"]
mod control_plane_store;

#[path = "commits.rs"]
mod commits;
mod commits_auth;
mod commits_auth_policy;

mod commits_decommission;
#[path = "commits_fleet.rs"]
mod commits_fleet;
mod commits_fleet_noop;
mod commits_generation;
mod commits_noop;
mod commits_pool;
mod commits_pool_noop;
mod retirement;
mod retirement_refs;
mod retirement_scan;

#[path = "mapping.rs"]
mod mapping;

#[path = "control_plane_pool.rs"]
mod control_plane_pool;
#[path = "lifecycle_impl.rs"]
mod lifecycle_impl;
#[path = "lifecycle_pool_admit.rs"]
mod lifecycle_pool_admit;
mod lifecycle_support;
mod listener_messages;
mod runner_lifetime;

#[path = "scan.rs"]
mod scan;
mod template_scan;

pub use scan::ScanReport;

mod auth_observations;

mod control_reads;
mod control_reads_more;

mod pool_admission;
