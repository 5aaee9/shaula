//! Exact Forgejo token resolution for the composition root.

use std::sync::Arc;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::forgejo::{ForgejoScope, ForgejoTarget};
use shaula_core::ports::forgejo::ForgejoPoolPort;
use shaula_core::registry::ControlPlaneStore;

pub(crate) async fn build_client(
    store: &Arc<dyn ControlPlaneStore>,
    profile_key: &str,
    revision: i64,
    target: &ForgejoTarget,
) -> CoreResult<Option<Arc<dyn ForgejoPoolPort>>> {
    target.validate()?;
    let Some(row) = store.auth_revision_get(profile_key, revision).await? else {
        return Ok(None);
    };
    if row.schema_version != 1 || row.kind != "forgejo_token" || row.state != "Active" {
        return Ok(None);
    }
    let stored_target = row
        .policy_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<ForgejoTarget>(json).ok());
    if stored_target.as_ref() != Some(target) {
        return Err(CoreError::new(
            ReasonCode::TargetNotAllowed,
            "Forgejo credential target mismatch",
        ));
    }
    let Some(bytes) = store.auth_credential_bytes(profile_key, revision).await? else {
        return Ok(None);
    };
    let token = String::from_utf8(bytes).map_err(|_| {
        CoreError::new(
            ReasonCode::CredentialMalformed,
            "Forgejo token is not valid UTF-8",
        )
    })?;
    if token.trim().is_empty() {
        return Err(CoreError::new(
            ReasonCode::CredentialMalformed,
            "Forgejo token is empty",
        ));
    }
    let scope = match &target.scope {
        ForgejoScope::Instance => shaula_forgejo::ForgejoScope::Instance,
        ForgejoScope::Organization { name } => {
            shaula_forgejo::ForgejoScope::Organization(name.clone())
        }
        ForgejoScope::User => shaula_forgejo::ForgejoScope::User,
        ForgejoScope::Repository { owner, name } => shaula_forgejo::ForgejoScope::Repository {
            owner: owner.clone(),
            name: name.clone(),
        },
    };
    let client = shaula_forgejo::ForgejoClient::new(&target.instance_url, token, scope)
        .map_err(|error| CoreError::new(ReasonCode::CredentialMalformed, error.to_string()))?;
    let client = if target.scope != ForgejoScope::Instance {
        let principal = row
            .validation_snapshot_json
            .as_deref()
            .and_then(|json| {
                serde_json::from_str::<shaula_core::ports::forgejo::ForgejoAuthProbe>(json).ok()
            })
            .and_then(|probe| {
                if target.scope == ForgejoScope::User {
                    probe.principal_id
                } else {
                    probe.target_id
                }
            })
            .filter(|id| *id > 0)
            .ok_or_else(|| {
                CoreError::new(
                    ReasonCode::AuthTargetDenied,
                    "Forgejo scope lacks a verified numeric identity",
                )
            })?;
        client
            .with_expected_scope_identity(principal)
            .map_err(|_| {
                CoreError::new(
                    ReasonCode::AuthTargetDenied,
                    "invalid Forgejo scope identity",
                )
            })?
    } else {
        client
    };
    Ok(Some(Arc::new(client)))
}
