//! Exact-credential resolution for the composition root, split from
//! `wiring.rs` to keep files within the 400-line budget (AGENTS.md).

use shaula_core::error::CoreResult;
use shaula_core::registry::ControlPlaneStore;
use shaula_core::secret::SecretString;
use std::sync::Arc;

/// Loads the exact accepted credential for the ACTIVE auth revision
/// (bytes never leave the store seam until this protected handoff).
/// A v2 revision routes through the frozen Account Binding of the
/// concrete fleet target — there is no profile-wide installation
/// (spec 0011 §4.2).
pub(crate) async fn build_credential(
    store: &Arc<dyn ControlPlaneStore>,
    profile_key: &str,
    revision: i64,
    target: &shaula_core::github::GitHubTarget,
) -> CoreResult<Option<shaula_scaleset::Credential>> {
    let Some(row) = store.auth_revision_get(profile_key, revision).await? else {
        return Ok(None);
    };
    if row.schema_version != 2 || row.kind != "github_app" {
        return Ok(None);
    }
    let Some(binding) = resolve_binding(store, profile_key, revision, target).await? else {
        return Ok(None);
    };
    let Some(bytes) = store.auth_credential_bytes(profile_key, revision).await? else {
        return Ok(None);
    };
    let secret = SecretString::new(String::from_utf8_lossy(&bytes).into_owned());
    Ok(Some(shaula_scaleset::Credential::GitHubApp {
        client_id: row.app_id.unwrap_or_default(),
        installation_id: binding.installation_id,
        private_key: secret,
    }))
}

/// Recheck frozen route convergence when constructing a client. A corrupt
/// revision cannot turn multiple matching routes into first-match authority.
async fn resolve_binding(
    store: &Arc<dyn ControlPlaneStore>,
    profile_key: &str,
    revision: i64,
    target: &shaula_core::github::GitHubTarget,
) -> CoreResult<Option<shaula_core::auth_context::AccountBinding>> {
    let Some(row) = store.auth_revision_get(profile_key, revision).await? else {
        return Ok(None);
    };
    let Some(policy) = row.target_policy()? else {
        return Ok(None);
    };
    let bindings = store.auth_bindings_get(profile_key, revision).await?;
    use shaula_core::auth_context::{resolve_desired_context, DesiredContextResolution};
    match resolve_desired_context(
        profile_key,
        revision,
        row.app_id.as_deref().unwrap_or_default(),
        &policy,
        &bindings,
        target,
    ) {
        DesiredContextResolution::Resolved(context) => Ok(bindings
            .into_iter()
            .find(|b| context.matches_ref_and_binding(profile_key, revision, b))),
        _ => Ok(None),
    }
}
