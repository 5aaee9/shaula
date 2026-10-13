//! GitLab Pool adapter for the Runner Backend seam (spec 0043). GitLab
//! `DELETE` does not interrupt a running job, but a registration whose token
//! is still held can acquire another one, so removal is pause → recheck →
//! delete. Ownership is the exact ID + description + `maintenance_note`.
//!
//! Failure modes this adapter must not turn into an unsafe removal:
//! 1. Recorded ID now names a different registration → `Unprovable`.
//! 2. Duplicate description in scope (GitLab allows it) → the note decides.
//! 3. Pause/read transport error → `Unavailable`, never assumed done.
//! 4. A job assigned while the pause lands → the recheck runs a pass later.
//! 5. `job_execution_status` lag after a job → `Busy` until it settles.
//! 6. An in-flight registration without identity → never proven absent.

use shaula_core::error::CoreResult;
use shaula_core::gitlab::{ownership_note, GitlabRegistrationRole};
use shaula_core::ports::gitlab::{GitlabPoolPort, GitlabRemovalOutcome, GitlabRunnerRef};
use shaula_core::registry::{
    ControlPlaneStore, FleetRuntimeGuard, GenerationRecord, LifecycleStore,
};

use super::{Registration, RemovalGate, RunnerRegistrations};
use crate::effect_gate::FleetEffectGates;

pub(crate) struct GitlabRegistrations<'a> {
    pub fleet_key: &'a str,
    pub guard: &'a FleetRuntimeGuard,
    pub lifecycle: &'a dyn LifecycleStore,
    pub store: &'a dyn ControlPlaneStore,
    pub gates: &'a FleetEffectGates,
    pub gitlab: &'a dyn GitlabPoolPort,
}

/// The exact remote registration behind a recorded identity, or the
/// verdict that ends the question.
enum Observed {
    Runner(GitlabRunnerRef),
    Verdict(Registration),
}

impl GitlabRegistrations<'_> {
    /// Removal authority exists only for the exact admitted Fleet authority.
    async fn authority_current(&self) -> CoreResult<bool> {
        Ok(self
            .store
            .fleet_get(self.fleet_key)
            .await?
            .is_some_and(|head| !head.tombstone && FleetRuntimeGuard::from(&head) == *self.guard))
    }

    async fn observe(&self, generation: &GenerationRecord) -> CoreResult<Observed> {
        let Some(id) = self
            .lifecycle
            .generation_gitlab_runner(&generation.id)
            .await?
        else {
            return Ok(Observed::Verdict(Registration::Unrecorded));
        };
        if !self.authority_current().await? {
            return Ok(Observed::Verdict(Registration::Unavailable));
        }
        let Ok(id) = u64::try_from(id) else {
            return Ok(Observed::Verdict(Registration::Unprovable));
        };
        Ok(match self.gitlab.get_runner(id).await {
            Err(_) => Observed::Verdict(Registration::Unavailable),
            Ok(None) => Observed::Verdict(Registration::Gone),
            // Lost update permission hides the note: retry, never misclassify.
            Ok(Some(remote)) if remote.ownership_hidden() => {
                Observed::Verdict(Registration::Unavailable)
            }
            Ok(Some(remote)) if is_owned_generation(&remote, self.fleet_key, generation) => {
                Observed::Runner(remote)
            }
            Ok(Some(_)) => Observed::Verdict(Registration::Unprovable),
        })
    }

    /// The GitLab busy-safe gate: pause, then prove on a LATER pass that no
    /// job is held. A job request that raced the pause can still be assigned,
    /// so the pausing pass never also rechecks (spec 0043 §6 step 2).
    async fn pause_and_recheck(&self, remote: &GitlabRunnerRef) -> Registration {
        if !remote.paused {
            return self.pause(remote.id).await;
        }
        match self.gitlab.running_jobs(remote.id).await {
            Err(_) => return Registration::Unavailable,
            Ok(None) => return Registration::Gone,
            Ok(Some(jobs)) if !jobs.is_empty() => return Registration::Busy,
            Ok(Some(_)) => {}
        }
        match self.gitlab.get_runner(remote.id).await {
            Err(_) => Registration::Unavailable,
            Ok(None) => Registration::Gone,
            // Unpaused out of band since the last pass: pause again and wait.
            Ok(Some(after)) if !after.paused => self.pause(after.id).await,
            Ok(Some(after)) if after.is_active() || !after.is_known() => Registration::Busy,
            Ok(Some(_)) => Registration::Removable,
        }
    }

    async fn pause(&self, id: u64) -> Registration {
        match self.gitlab.pause_runner(id).await {
            Ok(GitlabRemovalOutcome::Done) => Registration::Busy,
            Ok(GitlabRemovalOutcome::AlreadyAbsent) => Registration::Gone,
            Err(_) => Registration::Unavailable,
        }
    }
}

impl RunnerRegistrations for GitlabRegistrations<'_> {
    async fn admit_removal(
        &self,
        generation: &GenerationRecord,
        create_started: bool,
    ) -> CoreResult<Registration> {
        let _claim = self.gates.acquire_claim(self.fleet_key).await;
        let remote = match self.observe(generation).await? {
            Observed::Runner(remote) => remote,
            Observed::Verdict(verdict) => return Ok(verdict),
        };
        if create_started {
            return Ok(self.pause_and_recheck(&remote).await);
        }
        // No bootstrap was ever authorized, so no process holds the token.
        Ok(if remote.is_active() {
            Registration::Busy
        } else {
            Registration::Removable
        })
    }

    async fn remove(
        &self,
        generation: &GenerationRecord,
        gate: RemovalGate,
    ) -> CoreResult<Registration> {
        let _claim = self.gates.acquire_claim(self.fleet_key).await;
        let remote = match self.observe(generation).await? {
            Observed::Runner(remote) => remote,
            Observed::Verdict(verdict) => return Ok(verdict),
        };
        if gate
            == (RemovalGate::Retirement {
                create_started: true,
            })
        {
            match self.pause_and_recheck(&remote).await {
                Registration::Removable => {}
                verdict => return Ok(verdict),
            }
        }
        Ok(match self.gitlab.delete_runner(remote.id).await {
            Ok(GitlabRemovalOutcome::Done | GitlabRemovalOutcome::AlreadyAbsent) => {
                Registration::Gone
            }
            Err(_) => Registration::Unavailable,
        })
    }

    async fn prove_unrecorded_absent(
        &self,
        generation: &GenerationRecord,
    ) -> CoreResult<Registration> {
        // An unresolved registration intent may have landed without an identity.
        if self
            .lifecycle
            .operations_open_for_generation(&generation.id)
            .await?
            .iter()
            .any(|operation| operation.kind == "GitlabRegistration")
        {
            return Ok(Registration::Unprovable);
        }
        let _claim = self.gates.acquire_claim(self.fleet_key).await;
        if !self.authority_current().await? {
            return Ok(Registration::Unavailable);
        }
        let Ok(runners) = self.gitlab.list_runners(&generation.runner_name).await else {
            return Ok(Registration::Unavailable);
        };
        Ok(
            if runners
                .iter()
                .any(|runner| runner.description == generation.runner_name)
            {
                Registration::Unprovable
            } else {
                Registration::Gone
            },
        )
    }
}

/// The ownership marker is joint evidence with the description: GitLab
/// accepts duplicate descriptions in one scope.
pub(crate) fn is_owned_generation(
    runner: &GitlabRunnerRef,
    fleet_key: &str,
    generation: &GenerationRecord,
) -> bool {
    runner.description == generation.runner_name
        && runner.maintenance_note.as_deref()
            == Some(
                ownership_note(
                    fleet_key,
                    GitlabRegistrationRole::Generation,
                    &generation.runner_name,
                )
                .as_str(),
            )
}
