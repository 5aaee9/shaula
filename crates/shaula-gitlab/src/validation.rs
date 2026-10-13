//! Auth probe adapter: no durable validation or promotion policy.
use crate::{GitlabClient, GitlabError};
use shaula_core::{
    auth_validation::ProbeOutcome,
    error::{CoreError, CoreResult, ReasonCode},
    gitlab::GitlabTarget,
    ports::{gitlab::GitlabAuthProbe, AccessFailure},
    secret::SecretString,
};

pub async fn probe(
    target: &GitlabTarget,
    token: SecretString,
) -> CoreResult<Result<GitlabAuthProbe, ProbeOutcome>> {
    let client = GitlabClient::new(target.clone(), token)
        .map_err(|error| CoreError::new(ReasonCode::CredentialMalformed, error.to_string()))?;
    match client.probe_authentication().await {
        Ok(probe) => Ok(Ok(probe)),
        Err(GitlabError::UnsupportedServerVersion) => {
            Ok(Err(ProbeOutcome::Terminal("UnsupportedGitlabVersion")))
        }
        Err(error) => Ok(Err(match error.to_access_failure() {
            AccessFailure::Unauthenticated
            | AccessFailure::PermissionDenied
            | AccessFailure::TargetHiddenOrNotFound => ProbeOutcome::Terminal("Unauthenticated"),
            AccessFailure::RateLimited { retry_after } => ProbeOutcome::Retry {
                retry_after_ms: retry_after.and_then(|d| i64::try_from(d.as_millis()).ok()),
            },
            _ => ProbeOutcome::Retry {
                retry_after_ms: None,
            },
        })),
    }
}
