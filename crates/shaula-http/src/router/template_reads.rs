//! One Template Profile representation for list, detail and status reads.
use super::{require_scope, AppState};
use crate::problem::{mutation_problem, problem};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use shaula_core::registry::{Scope, TemplateProfileView};

pub(super) fn profile(view: &TemplateProfileView) -> serde_json::Value {
    serde_json::json!({
        "key": view.key,
        "incarnation": view.incarnation,
        "desiredRevision": view.desired_revision,
        "activeRevision": view.active_revision,
        "runnerBackend": view.runner_backend,
        "status": view.status,
        "platform": view.platform,
        "bindingsContract": view.bindings_contract,
        "bindings_present": view.bindings_present,
        "bindings": view.bindings,
        "validation": {"revision":view.desired_revision,"state":view.validation_state,"reason":view.validation_reason},
        "references": {"inUse":view.references_in_use}
    })
}

pub(super) async fn status(
    State(state): State<AppState>,
    Path(key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::TemplateRead) {
        return response;
    }
    match state.profiles.template_get(&auth.actor, &key).await {
        Ok(Ok(view)) => (StatusCode::OK, Json(profile(&view))).into_response(),
        Ok(Err(error)) => mutation_problem(&error).into_response(),
        Err(error) => {
            problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", error.summary).into_response()
        }
    }
}
