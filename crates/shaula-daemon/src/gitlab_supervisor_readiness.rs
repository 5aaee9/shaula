//! Readiness, claim detection and restart classification. A claim is the
//! registration's running job: its association is persisted before the
//! registration is paused, and a finished single-job registration retires.

use shaula_core::diagnostics::*;
use shaula_core::error::CoreResult;
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::gitlab::{GitlabRunnerRef, GitlabRunningJob};
use shaula_core::registry::{GenerationRecord, GitlabJobAssociation};

use super::GitlabPoolSupervisor;
use crate::runner_operation::gitlab::is_owned_generation;
use crate::supervisor::READINESS_TIMEOUT_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadinessNote {
    Silent,
    Satisfied,
    AwaitingOnline,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Advance(GenerationState, ReadinessNote),
    /// One running job: associate it, pause, then mark the Generation Busy.
    Claimed,
    Wait(ReadinessNote),
}

/// Pure readiness decision from the exact registration and its running jobs.
fn decide(
    state: GenerationState,
    observed: Result<Option<(&GitlabRunnerRef, &[GitlabRunningJob])>, ()>,
    created_at: i64,
    now: i64,
) -> Decision {
    use Decision::{Advance, Claimed, Wait};
    use GenerationState as G;
    let (runner, jobs) = match observed {
        Err(()) => return Advance(G::Quarantined, ReadinessNote::Silent),
        Ok(None) if state == G::WaitingOnline => {
            return Advance(G::CleanupRequired, ReadinessNote::Silent)
        }
        Ok(None) => return Advance(G::Retiring, ReadinessNote::Silent),
        Ok(Some(observed)) => observed,
    };
    if !runner.is_known() || jobs.len() > 1 {
        return Advance(G::Quarantined, ReadinessNote::Silent);
    }
    if !jobs.is_empty() {
        return if state == G::Busy && runner.paused {
            Wait(ReadinessNote::Satisfied)
        } else {
            Claimed
        };
    }
    match state {
        G::WaitingOnline if runner.is_online_idle() && !runner.paused => {
            Advance(G::Idle, ReadinessNote::Satisfied)
        }
        G::WaitingOnline
            if now.saturating_sub(created_at) >= READINESS_TIMEOUT_MS && !runner.is_active() =>
        {
            Advance(G::CleanupRequired, ReadinessNote::TimedOut)
        }
        G::WaitingOnline => Wait(ReadinessNote::AwaitingOnline),
        // Paused, offline or stale without a job: it can no longer serve.
        G::Idle if runner.status != "online" || runner.paused => {
            Advance(G::Retiring, ReadinessNote::Silent)
        }
        G::Idle => Wait(ReadinessNote::Satisfied),
        // The single job finished; `is_active` lags it briefly.
        G::Busy if !runner.is_active() => Advance(G::Retiring, ReadinessNote::Silent),
        _ => Wait(ReadinessNote::Silent),
    }
}

fn record_note(diagnostic: &mut Capture, note: ReadinessNote) {
    match note {
        ReadinessNote::Silent => {}
        ReadinessNote::Satisfied => {
            diagnostic.pass(StageId::RunnerInventory);
            diagnostic.outcome(Outcome::Satisfied);
        }
        ReadinessNote::AwaitingOnline => {
            diagnostic.reason(
                Code::LifecycleAwaitingOnline,
                StageId::RunnerInventory,
                false,
            );
            diagnostic.outcome(Outcome::Progressing);
        }
        ReadinessNote::TimedOut => {
            diagnostic.reason(
                Code::LifecycleReadinessTimeout,
                StageId::RunnerInventory,
                true,
            );
        }
    }
}

impl GitlabPoolSupervisor {
    pub(super) async fn reconcile_generation_readiness(
        &self,
        runners: &[GitlabRunnerRef],
        now: i64,
    ) -> CoreResult<()> {
        for generation in self
            .lifecycle
            .generations_for_fleet(&self.fleet_key)
            .await?
        {
            match generation.state {
                GenerationState::CreatePending => {
                    self.lifecycle
                        .generation_advance(&generation.id, GenerationState::Creating, now)
                        .await?;
                    self.lifecycle
                        .generation_advance(&generation.id, GenerationState::CleanupRequired, now)
                        .await?;
                }
                GenerationState::Creating => self.recover_create(&generation, now).await?,
                GenerationState::WaitingOnline | GenerationState::Idle | GenerationState::Busy => {
                    self.observe_readiness(&generation, runners, now).await?
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn observe_readiness(
        &self,
        generation: &GenerationRecord,
        runners: &[GitlabRunnerRef],
        now: i64,
    ) -> CoreResult<()> {
        let mut diagnostic = crate::diagnostic_capture::generation_capture(
            self.store.as_ref(),
            Some(&self.guard),
            generation,
            Lane::Readiness,
            QuestionId::Readiness,
            now,
        );
        let Some(id) = self
            .lifecycle
            .generation_gitlab_runner(&generation.id)
            .await?
            .and_then(|id| u64::try_from(id).ok())
        else {
            return self
                .advance(generation, GenerationState::Quarantined, now)
                .await;
        };
        // A duplicate description with another ID is contradictory evidence.
        if runners
            .iter()
            .any(|r| r.description == generation.runner_name && r.id != id)
        {
            return self
                .advance(generation, GenerationState::Quarantined, now)
                .await;
        }
        let read = async {
            let runner = match runners.iter().find(|r| r.id == id) {
                Some(runner) => Some(runner.clone()),
                None => self.gitlab.get_runner(id).await?,
            };
            let jobs = match &runner {
                Some(_) => self.gitlab.running_jobs(id).await?,
                None => None,
            };
            Ok::<_, shaula_core::ports::AccessFailure>((runner, jobs))
        };
        let Ok((runner, jobs)) = read.await else {
            diagnostic.reason(
                Code::ControlInventoryUnavailable,
                StageId::RunnerInventory,
                false,
            );
            return Ok(());
        };
        let observed = match (&runner, &jobs) {
            (Some(runner), _) if !is_owned_generation(runner, &self.fleet_key, generation) => {
                Err(())
            }
            (Some(runner), Some(jobs)) => Ok(Some((runner, jobs.as_slice()))),
            _ => Ok(None),
        };
        match decide(generation.state, observed, generation.created_at, now) {
            Decision::Advance(next, note) => {
                record_note(&mut diagnostic, note);
                self.advance(generation, next, now).await
            }
            Decision::Wait(note) => {
                record_note(&mut diagnostic, note);
                // A paused Busy registration still listing its job refreshes the projection.
                if let Some(job) = jobs.as_deref().and_then(<[GitlabRunningJob]>::first) {
                    self.project_running_job(generation, id, job, now).await;
                }
                Ok(())
            }
            Decision::Claimed => {
                record_note(&mut diagnostic, ReadinessNote::Satisfied);
                match jobs.as_deref().and_then(<[GitlabRunningJob]>::first) {
                    Some(job) => self.claim(generation, id, job, now).await,
                    None => Ok(()),
                }
            }
        }
    }

    /// Verified association first, then the pause: a lost pause is retried
    /// on the next tick, a lost association is never reconstructed.
    async fn claim(
        &self,
        generation: &GenerationRecord,
        runner_id: u64,
        job: &GitlabRunningJob,
        now: i64,
    ) -> CoreResult<()> {
        let association = GitlabJobAssociation {
            job_id: i64::try_from(job.id).unwrap_or(i64::MAX),
            project_id: i64::try_from(job.project_id).unwrap_or(i64::MAX),
            pipeline_id: i64::try_from(job.pipeline_id).unwrap_or(i64::MAX),
            observed_at: now,
        };
        if self
            .lifecycle
            .generation_set_gitlab_job(&generation.id, association)
            .await
            .is_err()
        {
            return self
                .advance(generation, GenerationState::Quarantined, now)
                .await;
        }
        // Best effort and after the association: never delays the pause.
        self.project_running_job(generation, runner_id, job, now)
            .await;
        if self.gitlab.pause_runner(runner_id).await.is_err() {
            tracing::warn!(fleet = %self.fleet_key, generation = %generation.id,
                "GitLab pause after claim failed; retrying on the next tick");
        }
        if generation.state == GenerationState::WaitingOnline {
            self.advance(generation, GenerationState::Idle, now).await?;
        }
        if generation.state != GenerationState::Busy {
            self.advance(generation, GenerationState::Busy, now).await?;
        }
        Ok(())
    }

    async fn advance(
        &self,
        generation: &GenerationRecord,
        next: GenerationState,
        now: i64,
    ) -> CoreResult<()> {
        self.lifecycle
            .generation_advance(&generation.id, next, now)
            .await
    }

    async fn recover_create(&self, generation: &GenerationRecord, now: i64) -> CoreResult<()> {
        let operations = self
            .lifecycle
            .operations_open_for_generation(&generation.id)
            .await?;
        let identity = self
            .lifecycle
            .generation_gitlab_runner(&generation.id)
            .await?;
        if let Some(operation) = operations.iter().find(|op| op.kind == "GitlabRegistration") {
            if identity.is_none() {
                return self
                    .classify_registration(generation, &operation.id, now)
                    .await;
            }
            self.lifecycle
                .operation_update_state(&operation.id, "Completed", now)
                .await?;
        }
        // A stored post-Create result is positive evidence the runtime returned;
        // otherwise cleanup must classify any partial apply from its own ledger.
        let next = if identity.is_some()
            && self
                .lifecycle
                .generation_state_identity(&generation.id)
                .await?
                .is_some()
        {
            for operation in operations.iter().filter(|op| op.kind == "Create") {
                self.lifecycle
                    .operation_update_state(&operation.id, "Completed", now)
                    .await?;
            }
            GenerationState::WaitingOnline
        } else {
            GenerationState::CleanupRequired
        };
        self.advance(generation, next, now).await
    }
}
