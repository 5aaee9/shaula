use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    ports::Clock,
    registry::ControlPlaneStore,
};
use std::sync::Arc;

pub(super) async fn validate(
    store: Arc<dyn ControlPlaneStore>,
    clock: Arc<dyn Clock>,
    key: String,
) -> CoreResult<()> {
    let Some(head) = store.auth_profile_get(&key).await? else {
        return Ok(());
    };
    if head.status != "Validating" {
        return Ok(());
    }
    let Some(row) = store.auth_revision_get(&key, head.desired_revision).await? else {
        return Ok(());
    };
    let allowlist: shaula_core::auth::TargetAllowlist =
        serde_json::from_str(&row.allowlist_json)
            .map_err(|_| CoreError::new(ReasonCode::Internal, "stored auth allowlist invalid"))?;
    let Some(credential) = crate::wiring::build_credential(&store, &key, row.revision).await?
    else {
        return Ok(());
    };
    let mut accepted = !allowlist.targets.is_empty();
    for target in allowlist.targets {
        let client =
            shaula_scaleset::ScalesetClient::production(target, credential.clone(), clock.clone())
                .map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "auth client construction failed")
                })?;
        if let Err(error) = client.validate_auth(&row).await {
            match error {
                shaula_scaleset::ScalesetError::Configuration { .. }
                | shaula_scaleset::ScalesetError::Status {
                    status: 401 | 403 | 404,
                    ..
                } => {
                    accepted = false;
                    break;
                }
                _ => {
                    return Err(CoreError::new(
                        ReasonCode::AccessVerificationFailed,
                        "auth validation temporarily unavailable",
                    ))
                }
            }
        }
    }
    store
        .auth_apply_validation(
            &key,
            row.revision,
            accepted,
            if accepted {
                None
            } else {
                Some("IdentityOrAccessVerificationFailed")
            },
            clock.now_unix_ms(),
        )
        .await
}
