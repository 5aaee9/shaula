//! Candidate predecessor loading (F2/R8): the continuity authority for
//! multi-account publications, split to keep files within the 400-line
//! budget (AGENTS.md).

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{AuthIdentityProof, AuthValidationSnapshot, ControlPlaneStore};
use std::sync::Arc;

/// The predecessor of the Candidate: the continuity authority loaded
/// INDEPENDENTLY of optional v2 snapshot state (F2/R8). A predecessor
/// that exists but cannot be trusted is fail-closed, never treated as a
/// first publication.
pub(crate) enum Predecessor {
    /// No active revision: first publication.
    First,
    /// A v2 predecessor: App identity plus proven numeric identities.
    V2 {
        app_id: String,
        identities: Vec<AuthIdentityProof>,
    },
}

/// Loads the predecessor. A store READ failure propagates as an error
/// (bounded retry) — it must never silently downgrade to
/// [`Predecessor::First`]; an unparseable v2 snapshot is
/// fail-closed corruption.
pub(crate) async fn load_predecessor(
    store: &Arc<dyn ControlPlaneStore>,
    key: &str,
) -> CoreResult<Predecessor> {
    let Some(active) = store.auth_revision_active(key).await? else {
        return Ok(Predecessor::First);
    };
    if active.schema_version != 2 || active.kind != "github_app" {
        return Err(CoreError::new(
            ReasonCode::CredentialMalformed,
            "unsupported authentication predecessor",
        ));
    }
    let app_id = active
        .app_id
        .filter(|id| id.parse::<i64>().is_ok_and(|id| id > 0))
        .ok_or_else(|| {
            CoreError::new(
                ReasonCode::CredentialMalformed,
                "active app identity invalid",
            )
        })?;
    let Some(json) = active.validation_snapshot_json.clone() else {
        return Err(CoreError::new(
            ReasonCode::Internal,
            "active v2 revision lacks its validation snapshot",
        ));
    };
    let snapshot: AuthValidationSnapshot = serde_json::from_str(&json).map_err(|e| {
        CoreError::new(
            ReasonCode::Internal,
            format!("validation snapshot unreadable: {e}"),
        )
    })?;
    Ok(Predecessor::V2 {
        app_id,
        identities: snapshot.identities,
    })
}
