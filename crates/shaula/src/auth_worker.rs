//! Concrete Auth adapters injected into the daemon's validation use cases.
use shaula_core::{
    auth_validation::{AuthValidationFactory, GitHubValidationPort, ProbeOutcome},
    error::CoreResult,
    forgejo::ForgejoTarget,
    ports::{forgejo::ForgejoAuthProbe, Clock},
    secret::SecretString,
};
use std::sync::Arc;

pub(crate) struct Adapters {
    pub endpoints: super::auth_worker_probe::WorkerEndpoints,
    pub clock: Arc<dyn Clock>,
}
#[async_trait::async_trait]
impl AuthValidationFactory for Adapters {
    fn github(&self) -> CoreResult<Box<dyn GitHubValidationPort>> {
        Ok(Box::new(
            shaula_scaleset::validation::GitHubValidation::new(
                self.endpoints.clone(),
                self.clock.clone(),
            )?,
        ))
    }
    async fn forgejo(
        &self,
        target: &ForgejoTarget,
        token: SecretString,
    ) -> CoreResult<Result<ForgejoAuthProbe, ProbeOutcome>> {
        shaula_forgejo::validation::probe(target, token).await
    }
}
