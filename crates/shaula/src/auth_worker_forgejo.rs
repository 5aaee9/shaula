//! Forgejo token validation for the composition root. The daemon remains
//! provider-agnostic; only this crate wires the Forgejo HTTP adapter.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::ports::{AccessFailure, Clock};
use shaula_core::registry::{AuthPromotionOutcome, AuthRevisionRow, ControlPlaneStore};
use shaula_forgejo::{ForgejoClient, ForgejoScope};
use std::sync::Arc;

pub(crate) async fn validate(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    row: &AuthRevisionRow,
) -> CoreResult<super::auth_worker::WorkerFlow> {
    let Some(target_json) = row.policy_json.as_deref() else {
        return reject(store, clock, key, row.revision, "CredentialMalformed").await;
    };
    let target: shaula_core::forgejo::ForgejoTarget = serde_json::from_str(target_json)
        .map_err(|error| CoreError::new(ReasonCode::CredentialMalformed, error.to_string()))?;
    let scope = match target.scope {
        shaula_core::forgejo::ForgejoScope::Instance => ForgejoScope::Instance,
        shaula_core::forgejo::ForgejoScope::Organization { name } => {
            ForgejoScope::Organization(name)
        }
        shaula_core::forgejo::ForgejoScope::User => ForgejoScope::User,
        shaula_core::forgejo::ForgejoScope::Repository { owner, name } => {
            ForgejoScope::Repository { owner, name }
        }
    };
    let Some(secret) = store.auth_credential_bytes(key, row.revision).await? else {
        return Ok(super::auth_worker::WorkerFlow::Deferred {
            retry_at_unix_ms: None,
        });
    };
    let token = String::from_utf8(secret).map_err(|_| {
        CoreError::new(
            ReasonCode::CredentialMalformed,
            "Forgejo token is not valid UTF-8",
        )
    })?;
    let client = ForgejoClient::new(&target.instance_url, token, scope)
        .map_err(|error| CoreError::new(ReasonCode::CredentialMalformed, error.to_string()))?;
    match client.probe_authentication().await {
        Ok(probe) => match store
            .auth_apply_validation_v2(
                key,
                row.revision,
                true,
                None,
                clock.now_unix_ms(),
                Some(shaula_core::registry::AuthPromotion {
                    bindings: Vec::new(),
                    snapshot_json: serde_json::to_string(&probe).map_err(|_| {
                        CoreError::new(ReasonCode::Internal, "Forgejo probe serialization failed")
                    })?,
                }),
            )
            .await?
        {
            AuthPromotionOutcome::Promoted | AuthPromotionOutcome::Rejected => {
                Ok(super::auth_worker::WorkerFlow::Done)
            }
            AuthPromotionOutcome::Restaged => Ok(super::auth_worker::WorkerFlow::Deferred {
                retry_at_unix_ms: None,
            }),
        },
        Err(shaula_forgejo::ForgejoError::UnsupportedServerVersion) => {
            reject(store, clock, key, row.revision, "UnsupportedForgejoVersion").await
        }
        Err(error) => match error.to_access_failure() {
            AccessFailure::Unauthenticated
            | AccessFailure::PermissionDenied
            | AccessFailure::TargetHiddenOrNotFound => {
                reject(store, clock, key, row.revision, "Unauthenticated").await
            }
            AccessFailure::RateLimited { retry_after } => {
                Ok(super::auth_worker::WorkerFlow::Deferred {
                    retry_at_unix_ms: retry_after
                        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
                        .map(|millis| clock.now_unix_ms().saturating_add(millis)),
                })
            }
            AccessFailure::Unavailable { .. } | AccessFailure::RequestUncertain { .. } => {
                Ok(super::auth_worker::WorkerFlow::Deferred {
                    retry_at_unix_ms: None,
                })
            }
            AccessFailure::SessionExpired => Ok(super::auth_worker::WorkerFlow::Deferred {
                retry_at_unix_ms: None,
            }),
        },
    }
}

async fn reject(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    revision: i64,
    reason: &str,
) -> CoreResult<super::auth_worker::WorkerFlow> {
    store
        .auth_apply_validation_v2(
            key,
            revision,
            false,
            Some(reason),
            clock.now_unix_ms(),
            None,
        )
        .await?;
    Ok(super::auth_worker::WorkerFlow::Done)
}
