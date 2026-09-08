//! Private read-only input projection; no artifact or binding material crosses this route.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use shaula_core::registry::{InputContractReadError, Scope};

use crate::problem::problem;
use crate::router::{require_scope, AppState};

pub(super) async fn get(
    State(state): State<AppState>,
    Path((key, revision)): Path<(String, i64)>,
    auth: crate::oidc::Authenticated,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::TemplateRead) {
        return response;
    }
    match state
        .profiles
        .template_input_contract_get(&auth.actor, &key, revision)
        .await
    {
        Ok(Ok(contract)) => Json(contract).into_response(),
        Ok(Err(InputContractReadError::Forbidden)) => problem(
            StatusCode::FORBIDDEN,
            "Forbidden",
            "template.read scope is required",
        )
        .into_response(),
        Ok(Err(InputContractReadError::NotFound)) => problem(
            StatusCode::NOT_FOUND,
            "NotFound",
            "template revision not found",
        )
        .into_response(),
        Ok(Err(InputContractReadError::Unavailable { reason })) => {
            problem(StatusCode::CONFLICT, "InputContractUnavailable", reason).into_response()
        }
        Err(_) => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal",
            "template input contract storage is unavailable",
        )
        .into_response(),
    }
}
