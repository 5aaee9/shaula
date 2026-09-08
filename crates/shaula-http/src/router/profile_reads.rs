//! R10-05 Profile read/retire route handlers, split to keep
//! profile_routes.rs within the 400-line limit (AGENTS.md).
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use shaula_core::registry::Scope;

use crate::problem::{mutation_problem, problem};
use crate::router::{accepted_response, idempotency_header, require_scope, AppState};

pub(crate) async fn profile_change_get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    match state.profiles.profile_change_get(&actor, &id).await {
        Ok(Some(change)) => {
            let scope = if change.resource_kind == "template_profile" {
                Scope::TemplateRead
            } else {
                Scope::AuthRead
            };
            if let Err(r) = require_scope(&actor, scope) {
                return r;
            }
            (StatusCode::OK, Json(change)).into_response()
        }
        Ok(None) => problem(
            StatusCode::NOT_FOUND,
            "NotFound",
            "profile change not found",
        )
        .into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_profile_list(
    State(state): State<AppState>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRead) {
        return response;
    }
    match state.profiles.template_list(&actor).await {
        Ok(views) => {
            let items: Vec<serde_json::Value> = views
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "key": v.key,
                        "incarnation": v.incarnation,
                        "desiredRevision": v.desired_revision,
                        "activeRevision": v.active_revision,
                        "status": v.status,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "profiles": items })),
            )
                .into_response()
        }
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_profile_delete(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRetire) {
        return response;
    }
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    match state
        .profiles
        .template_delete(
            &actor,
            &profile_key,
            idempotency_key,
            match super::parse_if_match(&headers) {
                Ok(value) => value,
                Err(response) => return response,
            },
        )
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn auth_profile_delete(
    State(state): State<AppState>,
    Path(key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
) -> Response {
    let actor = auth.actor;
    if let Err(r) = require_scope(&actor, Scope::AuthRetire) {
        return r;
    }
    let idem = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let expected = match super::parse_if_match(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state
        .profiles
        .auth_delete(&actor, &key, idem, expected)
        .await
    {
        Ok(Ok(value)) => accepted_response(&value),
        Ok(Err(e)) => mutation_problem(&e).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_revision_get(
    State(state): State<AppState>,
    Path((profile_key, revision)): Path<(String, i64)>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRead) {
        return response;
    }
    match state
        .profiles
        .template_revision_get(&actor, &profile_key, revision)
        .await
    {
        Ok(Ok(view)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "profileKey": view.profile_key,
                "revision": view.revision,
                "artifactDigest": view.artifact_digest,
                "engineRef": view.engine_ref,
                "platform": view.platform,
                "bindingsContract": view.bindings_contract,
                "state": view.state,
                "reason": view.reason,
                "bindingsPresent": view.bindings_present,
            })),
        )
            .into_response(),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_attestation_get(
    State(state): State<AppState>,
    Path((profile_key, revision, attestation_key)): Path<(String, i64, String)>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRead) {
        return response;
    }
    match state
        .profiles
        .attestation_get(&actor, &profile_key, revision, &attestation_key)
        .await
    {
        Ok(Ok(view)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "profileKey": view.profile_key,
                "revision": view.revision,
                "subject": view.subject,
                "result": view.result,
                "suite": {"name": view.suite.0, "version": view.suite.1},
                "completedAt": view.completed_at,
                "subjectVerified": view.subject_verified,
            })),
        )
            .into_response(),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn auth_profile_status(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::AuthRead) {
        return response;
    }
    match state.profiles.auth_get(&actor, &profile_key).await {
        Ok(Ok(view)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "key": view.key,
                "status": view.status,
                "desiredRevision": view.desired_revision,
                "activeRevision": view.active_revision,
            })),
        )
            .into_response(),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn auth_revision_get(
    State(state): State<AppState>,
    Path((profile_key, revision)): Path<(String, i64)>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::AuthRead) {
        return response;
    }
    match state
        .profiles
        .auth_revision_get(&actor, &profile_key, revision)
        .await
    {
        Ok(Ok(view)) => {
            // F9 (spec 0011 §6): historical reads use the versioned
            // rendering contract — v2 revisions carry schema_version, the
            // structured policy and non-secret bindings, not a legacy
            // installation projection.
            let mut body = serde_json::json!({
                "profileKey": view.profile_key,
                "revision": view.revision,
                "kind": view.kind,
                "appId": view.app_id,
                "installationId": view.installation_id,
                "patPrincipal": view.pat_principal,
                "schema_version": view.schema_version,
                "state": view.state,
                "reason": view.reason,
            });
            if view.schema_version >= 2 {
                body["target_policy"] = serde_json::json!(view.target_policy);
                body["bindings"] = serde_json::json!(view.bindings);
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}
