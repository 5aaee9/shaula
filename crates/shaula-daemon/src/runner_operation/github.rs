//! GitHub Actions Scale Set adapter for the Runner Backend seam. Removal uses
//! the exact Auth Revision that admitted the Generation; `JobStillRunning` is
//! the busy-safe gate.

use std::collections::HashMap;
use std::sync::Arc;

use shaula_core::error::CoreResult;
use shaula_core::ports::{GitHubAccessPort, RemovalOutcome, RunnerLookup};
use shaula_core::registry::{ControlPlaneStore, GenerationRecord, LifecycleStore};

use super::{Registration, RemovalGate, RunnerRegistrations};

pub(crate) struct GithubRegistrations<'a> {
    pub fleet_key: &'a str,
    pub lifecycle: &'a dyn LifecycleStore,
    pub store: &'a dyn ControlPlaneStore,
    /// The Auth Revision Ref this driver was admitted with.
    pub current: (&'a str, i64),
    pub github: &'a Arc<dyn GitHubAccessPort>,
    pub revision_clients: &'a HashMap<(String, i64), Arc<dyn GitHubAccessPort>>,
}

impl GithubRegistrations<'_> {
    /// The client for the exact Auth Revision that admitted the Generation.
    async fn admission_client(
        &self,
        generation: &GenerationRecord,
    ) -> CoreResult<Option<&Arc<dyn GitHubAccessPort>>> {
        let Some(reference) = self.store.auth_generation_ref(&generation.id).await? else {
            tracing::error!(generation = %generation.id, "generation auth reference missing");
            return Ok(None);
        };
        if reference.0 == self.current.0 && reference.1 == self.current.1 {
            return Ok(Some(self.github));
        }
        let client = self.revision_clients.get(&reference);
        if client.is_none() {
            tracing::error!(
                generation = %generation.id,
                auth_profile = %reference.0,
                auth_revision = reference.1,
                "generation auth client unavailable"
            );
        }
        Ok(client)
    }
}

impl RunnerRegistrations for GithubRegistrations<'_> {
    /// GitHub offers no read-only busy check: the removal call is the gate,
    /// and a `JobStillRunning` refusal leaves the one-job runner fenced.
    async fn admit_removal(
        &self,
        generation: &GenerationRecord,
        _create_started: bool,
    ) -> CoreResult<Registration> {
        if generation.github_runner_id.is_none() {
            return Ok(Registration::Unrecorded);
        }
        Ok(match self.admission_client(generation).await? {
            Some(_) => Registration::Removable,
            None => Registration::Unprovable,
        })
    }

    async fn remove(
        &self,
        generation: &GenerationRecord,
        _gate: RemovalGate,
    ) -> CoreResult<Registration> {
        let Some(runner_id) = generation.github_runner_id else {
            return Ok(Registration::Unrecorded);
        };
        let Some(github) = self.admission_client(generation).await? else {
            return Ok(Registration::Unprovable);
        };
        Ok(match github.remove_runner(runner_id).await {
            Ok(RemovalOutcome::Removed | RemovalOutcome::AlreadyAbsent) => Registration::Gone,
            // A disconnected job can remain busy remotely; never pretend it is gone.
            Ok(RemovalOutcome::JobStillRunning) => Registration::Busy,
            Err(_) => Registration::Unavailable,
        })
    }

    async fn prove_unrecorded_absent(
        &self,
        generation: &GenerationRecord,
    ) -> CoreResult<Registration> {
        let Some(scale_set_id) = self
            .lifecycle
            .scale_set_get(self.fleet_key)
            .await?
            .and_then(|row| row.scale_set_id)
        else {
            // Without the bound Scale Set the exact-name lookup cannot run.
            return Ok(Registration::Unavailable);
        };
        Ok(
            match self
                .github
                .get_runner_by_name(scale_set_id, &generation.runner_name)
                .await
            {
                Ok(RunnerLookup::None) => Registration::Gone,
                Ok(RunnerLookup::ExactlyOne(_) | RunnerLookup::Multiple) => {
                    Registration::Unprovable
                }
                Err(_) => Registration::Unavailable,
            },
        )
    }
}
