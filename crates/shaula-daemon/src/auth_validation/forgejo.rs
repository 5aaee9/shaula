//! Forgejo candidate validation and promotion through classified probe facts.

use shaula_core::auth_validation::{AuthValidationFactory, ProbeOutcome};
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::ports::Clock;
use shaula_core::registry::{AuthPromotionOutcome, AuthRevisionRow, ControlPlaneStore};
use std::sync::Arc;

pub(crate) async fn validate(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    row: &AuthRevisionRow,
    factory: &dyn AuthValidationFactory,
) -> CoreResult<super::WorkerFlow> {
    let Some(target_json) = row.policy_json.as_deref() else {
        return reject(store, clock, key, row.revision, "CredentialMalformed").await;
    };
    let target: shaula_core::forgejo::ForgejoTarget = serde_json::from_str(target_json)
        .map_err(|error| CoreError::new(ReasonCode::CredentialMalformed, error.to_string()))?;
    let Some(secret) = store.auth_credential_bytes(key, row.revision).await? else {
        return Ok(super::WorkerFlow::Deferred {
            retry_at_unix_ms: None,
        });
    };
    let token = String::from_utf8(secret).map_err(|_| {
        CoreError::new(
            ReasonCode::CredentialMalformed,
            "Forgejo token is not valid UTF-8",
        )
    })?;
    match factory
        .forgejo(&target, shaula_core::secret::SecretString::new(token))
        .await?
    {
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
                Ok(super::WorkerFlow::Done)
            }
            AuthPromotionOutcome::Restaged => Ok(super::WorkerFlow::Deferred {
                retry_at_unix_ms: None,
            }),
        },
        Err(ProbeOutcome::Terminal(reason)) => {
            reject(store, clock, key, row.revision, reason).await
        }
        Err(ProbeOutcome::Retry { retry_after_ms }) => Ok(super::WorkerFlow::Deferred {
            retry_at_unix_ms: retry_after_ms.map(|ms| clock.now_unix_ms().saturating_add(ms)),
        }),
        Err(ProbeOutcome::Proven) => Err(CoreError::new(
            ReasonCode::Internal,
            "Forgejo probe lacks identity facts",
        )),
    }
}

async fn reject(
    store: &Arc<dyn ControlPlaneStore>,
    clock: &Arc<dyn Clock>,
    key: &str,
    revision: i64,
    reason: &str,
) -> CoreResult<super::WorkerFlow> {
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
    Ok(super::WorkerFlow::Done)
}
