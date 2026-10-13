use async_trait::async_trait;
use shaula_core::ports::gitlab::{
    GitlabAnchorDemand, GitlabAuthProbe, GitlabPoolPort, GitlabRegistration, GitlabRemovalOutcome,
    GitlabRunnerRef, GitlabRunnerSpec, GitlabRunningJob,
};
use shaula_core::ports::{AccessFailure, EffectOutcome};

use crate::error::GitlabError;
use crate::GitlabClient;

fn map_error(error: GitlabError) -> AccessFailure {
    error.to_access_failure()
}

#[async_trait]
impl GitlabPoolPort for GitlabClient {
    async fn register_runner(
        &self,
        spec: &GitlabRunnerSpec,
    ) -> Result<EffectOutcome<GitlabRegistration>, AccessFailure> {
        match self.register_runner(spec).await {
            Ok(registration) => Ok(EffectOutcome::Definite(registration)),
            Err(GitlabError::RequestUncertain(summary)) => Ok(EffectOutcome::Uncertain { summary }),
            Err(error) => Err(map_error(error)),
        }
    }

    async fn list_runners(&self, prefix: &str) -> Result<Vec<GitlabRunnerRef>, AccessFailure> {
        self.list_runners(prefix).await.map_err(map_error)
    }

    async fn get_runner(&self, id: u64) -> Result<Option<GitlabRunnerRef>, AccessFailure> {
        self.get_runner(id).await.map_err(map_error)
    }

    async fn anchor_demand(&self, anchor_id: u64) -> Result<GitlabAnchorDemand, AccessFailure> {
        self.anchor_demand(anchor_id).await.map_err(map_error)
    }

    async fn running_jobs(
        &self,
        runner_id: u64,
    ) -> Result<Option<Vec<GitlabRunningJob>>, AccessFailure> {
        self.running_jobs(runner_id).await.map_err(map_error)
    }

    async fn pause_runner(&self, id: u64) -> Result<GitlabRemovalOutcome, AccessFailure> {
        self.pause_runner(id).await.map_err(map_error)
    }

    async fn delete_runner(&self, id: u64) -> Result<GitlabRemovalOutcome, AccessFailure> {
        self.delete_runner(id).await.map_err(map_error)
    }

    async fn auth_probe(&self) -> Result<GitlabAuthProbe, AccessFailure> {
        self.probe_authentication().await.map_err(map_error)
    }
}
