//! On-demand GitHub App installation navigation. Resolving a link never
//! publishes or changes the Profile's authorization policy.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use shaula_core::registry::{Actor, Scope};

use super::{require_scope, AppState};
use crate::problem::problem;

/// Public metadata only; the App JWT and private key stay in the adapter.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthInstallationLink {
    pub url: String,
    pub app_id: String,
    pub revision: i64,
    pub incarnation: String,
}

/// Stable failures carry no provider body, credential or transport diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthInstallationLinkError {
    ScopeDenied,
    NotFound,
    ProfileUnavailable,
    ProfileChanged,
    LookupUnavailable,
    InvalidApp,
}

/// Separate from Profile Registry admission: the composition root owns
/// the protected credential read and bounded, read-only GitHub request.
#[async_trait::async_trait]
pub trait AuthInstallationLinkPort: Send + Sync {
    async fn installation_link(
        &self,
        actor: &Actor,
        key: &str,
    ) -> Result<AuthInstallationLink, AuthInstallationLinkError>;
}

pub(super) async fn get(
    State(state): State<AppState>,
    Path(key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    for scope in [Scope::AuthRead, Scope::AuthWrite] {
        if let Err(response) = require_scope(&auth.actor, scope) {
            return response;
        }
    }
    let result = match &state.auth_installation_link {
        Some(links) => links.installation_link(&auth.actor, &key).await,
        None => Err(AuthInstallationLinkError::LookupUnavailable),
    };
    // The shared OIDC guard sets private/no-store for every response,
    // including rejected scopes and failures before this handler runs.
    match result {
        Ok(link) => axum::Json(link).into_response(),
        Err(error) => {
            use AuthInstallationLinkError as Error;
            let (status, code, detail) = match error {
                Error::ScopeDenied => (
                    StatusCode::FORBIDDEN,
                    "ScopeDenied",
                    "auth.read and auth.write are required",
                ),
                Error::NotFound => (
                    StatusCode::NOT_FOUND,
                    "NotFound",
                    "authentication profile not found",
                ),
                Error::ProfileUnavailable => (
                    StatusCode::CONFLICT,
                    "AuthProfileUnavailable",
                    "an active GitHub App profile is required",
                ),
                Error::ProfileChanged => (
                    StatusCode::CONFLICT,
                    "AuthProfileChanged",
                    "authentication profile changed; refresh before trying again",
                ),
                Error::LookupUnavailable => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "GitHubAppUnavailable",
                    "the GitHub App installation link is temporarily unavailable; try again",
                ),
                Error::InvalidApp => (
                    StatusCode::BAD_GATEWAY,
                    "GitHubAppInvalid",
                    "the active GitHub App could not be verified; check its credential",
                ),
            };
            problem(status, code, detail).into_response()
        }
    }
}
