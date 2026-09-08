//! Non-secret source and variable discovery through the artifact library.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use shaula_core::error::{CoreError, ReasonCode};
use shaula_core::registry::Scope;

use super::{require_scope, AppState};
use crate::oidc::Authenticated;
use crate::problem::problem;

fn read_failure(error: CoreError) -> Response {
    if error.code == ReasonCode::TemplateInvalid || error.code == ReasonCode::SpecInvalid {
        problem(
            StatusCode::CONFLICT,
            "VariablesUnavailable",
            "template variable declarations are unsupported or inconsistent with their schemas",
        )
        .into_response()
    } else {
        problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal",
            "template library storage is unavailable",
        )
        .into_response()
    }
}

pub(super) async fn sources(State(state): State<AppState>, auth: Authenticated) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::TemplateRead) {
        return response;
    }
    match state.artifact_publisher.sources().await {
        Ok(sources) => Json(serde_json::json!({ "sources": sources })).into_response(),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn variables(
    State(state): State<AppState>,
    Path(digest): Path<String>,
    auth: Authenticated,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::TemplateRead) {
        return response;
    }
    if shaula_core::artifact_layout::artifact_dir(std::path::Path::new("."), &digest).is_none() {
        return problem(
            StatusCode::NOT_FOUND,
            "NotFound",
            "template artifact not found",
        )
        .into_response();
    }
    match state.artifact_publisher.variables(&digest).await {
        Ok(Some(variables)) => Json(variables).into_response(),
        Ok(None) => problem(
            StatusCode::NOT_FOUND,
            "NotFound",
            "template artifact not found",
        )
        .into_response(),
        Err(error) => read_failure(error),
    }
}
