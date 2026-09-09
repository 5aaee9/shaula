//! Auth Profile read routes (spec 0011 §6), split from
//! profile_routes.rs to keep files within the 400-line budget.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use shaula_core::registry::Scope;

use crate::problem::{mutation_problem, problem};
use crate::router::{require_scope, resource_version_headers, AppState};

pub(crate) async fn auth_profile_list(
    State(state): State<AppState>,
    auth: crate::oidc::Authenticated,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::AuthRead) {
        return response;
    }
    match state.profiles.auth_list(&auth.actor).await {
        Ok(views) => axum::Json(serde_json::json!({
            "profiles": views.iter().map(auth_profile_body).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

/// Current dependency inventory for an explicit policy preview.
pub(crate) async fn auth_profile_impact(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::AuthRead) {
        return response;
    }
    match state.profiles.auth_get(&auth.actor, &profile_key).await {
        Ok(Ok(view)) => axum::Json(serde_json::json!({
            "desiredRevision": view.desired_revision,
            "liveFleets": view.live_fleets.iter().map(|fleet| serde_json::json!({
                "fleetKey": fleet.fleet_key, "phase": fleet.phase, "target": fleet.target,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn auth_profile_get(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::AuthRead) {
        return response;
    }
    match state.profiles.auth_get(&actor, &profile_key).await {
        Ok(Ok(view)) => {
            let body = auth_profile_body(&view);
            (
                StatusCode::OK,
                resource_version_headers(&format!(
                    "{}:{}",
                    view.incarnation, view.desired_revision
                )),
                axum::Json(body),
            )
                .into_response()
        }
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

fn render_binding(
    binding: &shaula_core::auth_context::AccountBinding,
    state: &shaula_core::registry::AuthRevisionState,
) -> serde_json::Value {
    let health = state.binding_health.iter().find(|health| {
        health.account_id == binding.account_id && health.installation_id == binding.installation_id
    });
    serde_json::json!({
        "account_id": binding.account_id, "account_kind": binding.account_kind,
        "login": binding.login, "installation_id": binding.installation_id,
        "repository_selection": binding.repository_selection, "validated_at_ms": binding.validated_at_ms,
        "health": health.map_or("Unknown", |health| health.state.as_str()),
        "reason": health.and_then(|health| health.reason.as_deref()).unwrap_or("NoFreshAccessObservation"),
        "checked_at_ms": health.and_then(|health| health.checked_at_ms),
        "valid_until_ms": health.and_then(|health| health.valid_until_ms),
        "affected_fleets": health.map(|health| &health.affected_fleets).cloned().unwrap_or_default(),
    })
}

fn auth_profile_body(view: &shaula_core::registry::AuthProfileView) -> serde_json::Value {
    use shaula_core::registry::AuthRevisionState;
    let render_state = |state: &AuthRevisionState| {
        if state.schema_version == 2 && state.state != "Unsupported" {
            serde_json::json!({
                "revision": state.revision,
                "schema_version": state.schema_version,
                "state": state.state,
                "reason": state.reason,
                "app_id": state.app_id,
                "target_policy": state.target_policy.as_ref().map(|p| p.selectors()),
                "bindings": state.bindings.iter().map(|binding| render_binding(binding, state))
                    .collect::<Vec<_>>(),
            })
        } else {
            serde_json::json!({
                "revision": state.revision,
                "schema_version": state.schema_version,
                "state": "Unsupported",
                "reason": "UnsupportedAuthenticationFormat",
            })
        }
    };
    let supported = view.schema_version == 2
        && view.kind == Some(shaula_core::auth::AuthKind::GithubApp)
        && view
            .active
            .as_ref()
            .is_none_or(|active| active.schema_version == 2 && active.state != "Unsupported");
    let mut body = serde_json::json!({
        "key": view.key,
        "incarnation": view.incarnation,
        "desiredRevision": view.desired_revision,
        "activeRevision": view.active_revision,
        "status": if supported { view.status.as_str() } else { "Unsupported" },
        "kind": view.kind.map(shaula_core::auth::AuthKind::as_str),
        "credential_present": view.credential_present,
        "schema_version": view.schema_version,
        "liveFleets": view.live_fleets.iter().map(|fleet| serde_json::json!({
            "fleetKey": fleet.fleet_key, "phase": fleet.phase, "target": fleet.target,
        })).collect::<Vec<_>>(),
    });
    if supported {
        body["app_id"] = serde_json::json!(view.app_id);
    }
    if let Some(active) = &view.active {
        body["active"] = render_state(active);
    }
    if let Some(desired) = &view.desired {
        body["desired"] = render_state(desired);
    }
    body
}
#[cfg(test)]
#[path = "profile_auth_reads_tests.rs"]
mod tests;
