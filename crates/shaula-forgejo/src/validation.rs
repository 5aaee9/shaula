//! Auth probe adapter: no durable validation or promotion policy.
use crate::{ForgejoClient, ForgejoError, ForgejoScope};
use shaula_core::{
    auth_validation::ProbeOutcome,
    error::{CoreError, CoreResult, ReasonCode},
    forgejo::ForgejoTarget,
    ports::{forgejo::ForgejoAuthProbe, AccessFailure},
    secret::SecretString,
};
pub async fn probe(
    target: &ForgejoTarget,
    token: SecretString,
) -> CoreResult<Result<ForgejoAuthProbe, ProbeOutcome>> {
    let scope = match target.scope.clone() {
        shaula_core::forgejo::ForgejoScope::Instance => ForgejoScope::Instance,
        shaula_core::forgejo::ForgejoScope::Organization { name } => {
            ForgejoScope::Organization(name)
        }
        shaula_core::forgejo::ForgejoScope::User => ForgejoScope::User,
        shaula_core::forgejo::ForgejoScope::Repository { owner, name } => {
            ForgejoScope::Repository { owner, name }
        }
    };

    let client = ForgejoClient::new(&target.instance_url, token.expose().to_owned(), scope)
        .map_err(|error| CoreError::new(ReasonCode::CredentialMalformed, error.to_string()))?;
    match client.probe_authentication().await {
        Ok(probe) => Ok(Ok(probe)),
        Err(ForgejoError::UnsupportedServerVersion) => {
            Ok(Err(ProbeOutcome::Terminal("UnsupportedForgejoVersion")))
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
