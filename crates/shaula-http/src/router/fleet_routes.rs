//! Fleet route handlers: conditional writes, status, changes.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use shaula_core::registry::Scope;

use crate::dto::FleetSpecDto;
use crate::problem::{mutation_problem, problem};
use crate::router::{
    accepted_response, idempotency_header, if_none_match_star, parse_if_match, require_scope,
    AppState,
};

pub(crate) async fn fleet_put(
    State(state): State<AppState>,
    Path(fleet_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::FleetWrite) {
        return response;
    }
    let Ok(spec_dto) = serde_json::from_slice::<FleetSpecDto>(&body) else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "fleet spec rejected by strict validation",
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
        .fleets
        .fleet_put(
            &actor,
            &fleet_key,
            spec_dto.into_domain(),
            create,
            if_match,
            idempotency_key,
        )
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn fleet_get(
    State(state): State<AppState>,
    Path(fleet_key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::FleetRead) {
        return response;
    }
    match state.fleets.fleet_get(&actor, &fleet_key).await {
        Ok(Ok(resource)) => {
            let etag = format!("\"{}:{}\"", resource.incarnation, resource.revision);
            let spec_json = serde_json::to_value(&resource.spec).unwrap_or(serde_json::Value::Null);
            let resolved_template = resource.resolved_template.as_ref().map(|t| {
                serde_json::json!({
                    "key": t.0,
                    "revision": t.1,
                    "artifactDigest": t.2,
                    "attestationId": t.3,
                })
            });
            let body = serde_json::json!({
                "key": resource.key,
                "spec": spec_json,
                "metadata": {
                    "incarnation": resource.incarnation,
                    "revision": resource.revision,
                },
                "resolved": {
                    "template": resolved_template,
                    "authDesired": {
                        "profileKey": resource.resolved_auth.0,
                        "revision": resource.resolved_auth.1,
                    }
                }
            });
            ([(axum::http::header::ETAG, etag)], Json(body)).into_response()
        }
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn fleet_status(
    State(state): State<AppState>,
    Path(fleet_key): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::FleetRead) {
        return response;
    }
    match state.fleets.fleet_status_get(&actor, &fleet_key).await {
        Ok(Ok(status)) => {
            let desired_auth = status.dependencies.auth.as_ref().map(|auth| {
                serde_json::json!({
                    "desired": {"profileKey": auth.desired.0, "revision": auth.desired.1},
                    "observed": auth.observed.as_ref().map(|o| serde_json::json!({"profileKey": o.0, "revision": o.1})),
                    "handoffState": auth.handoff_state,
                })
            });
            let body = serde_json::json!({
                "fleetKey": status.fleet_key,
                "desiredRevision": status.desired_revision,
                "observedRevision": status.observed_revision,
                "phase": status.phase,
                "githubAuth": desired_auth,
                "conditions": status.conditions.iter().map(|c| serde_json::json!({
                    "type": c.condition_type,
                    "status": c.status,
                    "reason": c.reason,
                })).collect::<Vec<_>>(),
                "capacity": {
                    "assignedDemand": status.capacity.assigned_demand,
                    "target": status.capacity.target,
                    "effective": status.capacity.effective,
                    "occupancy": status.capacity.occupancy,
                },
                "lastError": status.last_error,
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn fleet_delete(
    State(state): State<AppState>,
    Path(fleet_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::FleetRetire) {
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
            "If-Match required for decommission",
        )
        .into_response();
    }
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    match state
        .fleets
        .fleet_delete(&actor, &fleet_key, if_match, idempotency_key)
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(crate) async fn fleet_change_get(
    State(state): State<AppState>,
    Path(change_id): Path<String>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::FleetRead) {
        return response;
    }
    match state.fleets.fleet_change_get(&actor, &change_id).await {
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

/// R10-05: the fleet LIST route — every active (non-tombstoned) fleet
/// with its desired revision and incarnation.
pub(crate) async fn fleet_list(
    State(state): State<AppState>,
    auth: crate::oidc::Authenticated,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::FleetRead) {
        return response;
    }
    match state.fleets.fleet_list(&actor).await {
        Ok(fleets) => {
            let items: Vec<serde_json::Value> = fleets
                .iter()
                .map(|(key, revision, incarnation)| {
                    serde_json::json!({
                        "key": key,
                        "revision": revision,
                        "incarnation": incarnation,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "fleets": items }))).into_response()
        }
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}
