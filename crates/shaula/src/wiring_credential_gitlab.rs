//! Exact GitLab token resolution for the composition root. The client is
//! pinned to the numeric scope ID proven at validation, so a re-pointed
//! group/project path fails closed instead of managing someone else's runners.

use std::sync::Arc;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::gitlab::GitlabTarget;
use shaula_core::ports::gitlab::{GitlabAuthProbe, GitlabPoolPort};
use shaula_core::registry::ControlPlaneStore;
use shaula_core::secret::SecretString;

pub(crate) async fn build_client(
    store: &Arc<dyn ControlPlaneStore>,
    profile_key: &str,
    revision: i64,
    target: &GitlabTarget,
) -> CoreResult<Option<Arc<dyn GitlabPoolPort>>> {
    target.validate()?;
    let Some(row) = store.auth_revision_get(profile_key, revision).await? else {
        return Ok(None);
    };
    if !row.is_gitlab_token_v1() || row.state != "Active" {
        return Ok(None);
    }
    let stored_target = row
        .policy_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<GitlabTarget>(json).ok());
    if stored_target.as_ref() != Some(target) {
        return Err(CoreError::new(
            ReasonCode::TargetNotAllowed,
            "GitLab credential target mismatch",
        ));
    }
    let scope_id = row
        .validation_snapshot_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<GitlabAuthProbe>(json).ok())
        .map(|probe| probe.target_id)
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            CoreError::new(
                ReasonCode::AuthTargetDenied,
                "GitLab scope lacks a verified numeric identity",
            )
        })?;
    let Some(bytes) = store.auth_credential_bytes(profile_key, revision).await? else {
        return Ok(None);
    };
    let token = String::from_utf8(bytes).map_err(|_| {
        CoreError::new(
            ReasonCode::CredentialMalformed,
            "GitLab token is not valid UTF-8",
        )
    })?;
    if token.trim().is_empty() {
        return Err(CoreError::new(
            ReasonCode::CredentialMalformed,
            "GitLab token is empty",
        ));
    }
    let client = shaula_gitlab::GitlabClient::new(target.clone(), SecretString::new(token))
        .and_then(|client| client.with_expected_scope_id(scope_id))
        .map_err(|_| {
            CoreError::new(
                ReasonCode::CredentialMalformed,
                "GitLab client configuration rejected",
            )
        })?;
    Ok(Some(Arc::new(client)))
}
