//! Shared TemplatePool route handlers (spec 0037): conditional writes
//! and reads riding the template permission family.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use shaula_core::registry::Scope;
use shaula_core::template_pool::TemplatePoolSpec;

use crate::problem::{mutation_problem, problem};
use crate::router::{
    accepted_response, idempotency_header, if_none_match_star, parse_if_match, require_scope,
    resource_version_headers, AppState,
};

pub(crate) async fn template_pool_put(
    State(state): State<AppState>,
    Path(pool_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplatePublish) {
        return response;
    }
    let Ok(spec) = serde_json::from_slice::<TemplatePoolSpec>(&body) else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "template pool spec rejected by strict validation",
        )
        .into_response();
    };
    let create = if_none_match_star(&headers);
    let if_match = match parse_if_match(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if !create && if_match.is_none() {
        return problem(
            StatusCode::PRECONDITION_REQUIRED,
            "PreconditionRequired",
            "If-None-Match: * or If-Match required",
        )
        .into_response();
    }
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    match state
        .pools
        .template_pool_put(&actor, &pool_key, spec, create, if_match, idempotency_key)
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_pool_get(
    State(state): State<AppState>,
    Path(pool_key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRead) {
        return response;
    }
    match state.pools.template_pool_get(&actor, &pool_key).await {
        Ok(Ok(resource)) => {
            let version = format!("{}:{}", resource.incarnation, resource.revision);
            let body = serde_json::json!({
                "key": resource.key,
                "spec": serde_json::to_value(&resource.spec).unwrap_or(serde_json::Value::Null),
                "metadata": {
                    "incarnation": resource.incarnation,
                    "revision": resource.revision,
                },
                "resolved": {
                    "members": resource.resolved_members.iter().map(|m| serde_json::json!({
                        "key": m.key,
                        "templateProfileKey": m.template_profile_key,
                        "templateRevision": m.template_revision,
                        "templateArtifactDigest": m.template_artifact_digest,
                        "templateAttestationId": m.template_attestation_id,
                        "templateInputs": m.template_inputs,
                        "inputsDigest": m.inputs_digest,
                        "weight": m.weight,
                        "maxRunners": m.max_runners,
                    })).collect::<Vec<_>>(),
                }
            });
            (resource_version_headers(&version), Json(body)).into_response()
        }
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_pool_delete(
    State(state): State<AppState>,
    Path(pool_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRetire) {
        return response;
    }
    let if_match = match parse_if_match(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if if_match.is_none() {
        return problem(
            StatusCode::PRECONDITION_REQUIRED,
            "PreconditionRequired",
            "If-Match required for pool deletion",
        )
        .into_response();
    }
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    match state
        .pools
        .template_pool_delete(&actor, &pool_key, if_match, idempotency_key)
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

/// Pool list: every active (non-tombstoned) pool with its desired
/// revision and incarnation.
pub(crate) async fn template_pool_list(
    State(state): State<AppState>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRead) {
        return response;
    }
    match state.pools.template_pool_list(&actor).await {
        Ok(pools) => {
            let items: Vec<serde_json::Value> = pools
                .iter()
                .map(|(key, revision, incarnation)| {
                    serde_json::json!({
                        "key": key,
                        "revision": revision,
                        "incarnation": incarnation,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "pools": items }))).into_response()
        }
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn template_pool_change_get(
    State(state): State<AppState>,
    Path(change_id): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::TemplateRead) {
        return response;
    }
    match state
        .pools
        .template_pool_change_get(&actor, &change_id)
        .await
    {
        Ok(Some(change)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": change.id,
                "resourceKind": change.resource_kind,
                "resourceKey": change.resource_key,
                "revision": change.revision,
                "kind": change.kind,
                "state": change.state,
                "reason": change.reason,
            })),
        )
            .into_response(),
        Ok(None) => problem(StatusCode::NOT_FOUND, "NotFound", "change not found").into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}
