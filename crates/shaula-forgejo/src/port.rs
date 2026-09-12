use async_trait::async_trait;
use shaula_core::ports::forgejo::{
    ForgejoAuthProbe, ForgejoJob, ForgejoPoolPort, ForgejoRegistration,
    ForgejoRegistrationUncertainty, ForgejoRemovalOutcome, ForgejoRunnerRef,
};
use shaula_core::ports::{AccessFailure, EffectOutcome};

use crate::error::ForgejoError;
use crate::models::{Job, Registration, RegistrationUncertainty, Removal, Runner};
use crate::ForgejoClient;

impl From<Runner> for ForgejoRunnerRef {
    fn from(runner: Runner) -> Self {
        Self {
            id: runner.id,
            uuid: runner.uuid,
            name: runner.name,
            status: runner.status,
            labels: runner.labels.into_iter().map(|label| label.name).collect(),
            ephemeral: runner.ephemeral,
            version: runner.version,
        }
    }
}

impl From<Job> for ForgejoJob {
    fn from(job: Job) -> Self {
        Self {
            id: job.id,
            handle: job.handle,
            attempt: job.attempt,
            status: job.status,
            runs_on: job.runs_on,
            task_id: job.task_id,
            run_id: job.run_id,
            repo_id: job.repo_id,
            name: job.name,
        }
    }
}

impl From<Registration> for ForgejoRegistration {
    fn from(registration: Registration) -> Self {
        Self {
            id: registration.id,
            uuid: registration.uuid,
            token: registration.token,
        }
    }
}

fn map_uncertainty(value: RegistrationUncertainty) -> ForgejoRegistrationUncertainty {
    match value {
        RegistrationUncertainty::None => ForgejoRegistrationUncertainty::None,
        RegistrationUncertainty::ExactlyOneCleanupRequired => {
            ForgejoRegistrationUncertainty::ExactlyOneCleanupRequired
        }
        RegistrationUncertainty::Quarantined => ForgejoRegistrationUncertainty::Quarantined,
    }
}

fn map_removal(value: Removal) -> ForgejoRemovalOutcome {
    match value {
        Removal::Removed => ForgejoRemovalOutcome::Removed,
        Removal::AlreadyAbsent => ForgejoRemovalOutcome::AlreadyAbsent,
        Removal::Busy => ForgejoRemovalOutcome::Busy,
    }
}

fn map_error(error: ForgejoError) -> AccessFailure {
    error.to_access_failure()
}

#[async_trait]
impl ForgejoPoolPort for ForgejoClient {
    async fn register_runner(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<EffectOutcome<ForgejoRegistration>, AccessFailure> {
        match self.register_runner(name, description).await {
            Ok(registration) => Ok(EffectOutcome::Definite(registration.into())),
            Err(ForgejoError::RequestUncertain(summary)) => {
                Ok(EffectOutcome::Uncertain { summary })
            }
            Err(error) => Err(map_error(error)),
        }
    }

    async fn list_runners(&self) -> Result<Vec<ForgejoRunnerRef>, AccessFailure> {
        self.list_runners()
            .await
            .map(|runners| runners.into_iter().map(Into::into).collect())
            .map_err(map_error)
    }

    async fn get_runner(&self, id: u64) -> Result<Option<ForgejoRunnerRef>, AccessFailure> {
        self.get_runner(id)
            .await
            .map(|runner| runner.map(Into::into))
            .map_err(map_error)
    }

    async fn list_jobs(&self, labels: &[String]) -> Result<Vec<ForgejoJob>, AccessFailure> {
        self.list_jobs(labels)
            .await
            .map(|jobs| jobs.into_iter().map(Into::into).collect())
            .map_err(map_error)
    }

    async fn delete_runner(&self, id: u64) -> Result<ForgejoRemovalOutcome, AccessFailure> {
        self.delete_runner(id)
            .await
            .map(map_removal)
            .map_err(|error| match error {
                ForgejoError::RequestUncertain(summary) => {
                    AccessFailure::RequestUncertain { summary }
                }
                other => map_error(other),
            })
    }

    async fn classify_uncertain_registration(
        &self,
        name: &str,
        labels: &[String],
    ) -> Result<ForgejoRegistrationUncertainty, AccessFailure> {
        self.classify_uncertain_registration(name, labels)
            .await
            .map(map_uncertainty)
            .map_err(map_error)
    }

    async fn uncertain_registration_candidates(
        &self,
        name: &str,
        labels: &[String],
    ) -> Result<Vec<ForgejoRunnerRef>, AccessFailure> {
        self.uncertain_registration_candidates(name, labels)
            .await
            .map(|runners| runners.into_iter().map(Into::into).collect())
            .map_err(map_error)
    }

    async fn auth_probe(&self) -> Result<ForgejoAuthProbe, AccessFailure> {
        self.probe_authentication().await.map_err(map_error)
    }
}
