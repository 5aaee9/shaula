//! Fleet route handlers: conditional writes, status, changes.

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use shaula_core::registry::Scope;

use crate::dto::FleetSpecDto;
use crate::problem::{problem, registry_response};
use crate::router::key_page::{ListQuery, RequestPage};
use crate::router::{
    accepted_response, idempotency_header, put_preconditions, require_if_match, require_scope,
    resource_version_headers, AppState,
};

pub(super) fn resolved_pool_member_json(
    member: &shaula_core::template_pool::ResolvedTemplatePoolMember,
) -> serde_json::Value {
    serde_json::json!({
        "key": member.key,
        "templateProfileKey": member.template_profile_key,
        "templateRevision": member.template_revision,
        "templateArtifactDigest": member.template_artifact_digest,
        "templateAttestationId": member.template_attestation_id,
        "templateInputs": member.template_inputs,
        "inputsDigest": member.inputs_digest,
        "weight": member.weight,
        "maxRunners": member.max_runners,
    })
}

pub(super) fn change_response(
    result: shaula_core::error::CoreResult<Option<shaula_core::registry::ChangeView>>,
) -> Response {
    match result {
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
    let (create, if_match) = match put_preconditions(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    registry_response(
        state
            .fleets
            .fleet_put(
                &actor,
                &fleet_key,
                spec_dto.into_domain(),
                create,
                if_match,
                idempotency_key,
            )
            .await,
        |accepted| accepted_response(&accepted),
    )
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
    registry_response(
        state.fleets.fleet_get(&actor, &fleet_key).await,
        |resource| {
            let version = format!("{}:{}", resource.incarnation, resource.revision);
            let spec_json = serde_json::to_value(&resource.spec).unwrap_or(serde_json::Value::Null);
            let resolved_template = resource.resolved_template.as_ref().map(|t| {
                serde_json::json!({
                    "key": t.0,
                    "revision": t.1,
                    "artifactDigest": t.2,
                    "attestationId": t.3,
                })
            });
            let resolved_template_pool = (!resource.resolved_template_pool.is_empty()).then(|| {
                resource
                    .resolved_template_pool
                    .iter()
                    .map(resolved_pool_member_json)
                    .collect::<Vec<_>>()
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
                    "templatePool": resolved_template_pool,
                    "authDesired": {
                        "profileKey": resource.resolved_auth.0,
                        "revision": resource.resolved_auth.1,
                    }
                }
            });
            (resource_version_headers(&version), Json(body)).into_response()
        },
    )
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
    registry_response(
        state.fleets.fleet_status_get(&actor, &fleet_key).await,
        |status| {
            let desired_auth = status.dependencies.auth.as_ref().map(|auth| {
                let mut auth_json = serde_json::json!({
                    "desired": {"profileKey": auth.desired.0, "revision": auth.desired.1},
                    "observed": auth.observed.as_ref().map(|o| serde_json::json!({"profileKey": o.0, "revision": o.1})),
                    "handoffState": auth.handoff_state,
                });
                // Exact Resolved Auth Context rollout (spec 0011 §6): the
                // fleet page shows the per-target resolution result — the
                // profile's Active status never substitutes for it.
                if let Some(context) = &auth.context {
                    auth_json["context"] = serde_json::json!({
                        "desired": context.desired.as_ref().map(|(k, r)| serde_json::json!({"profileKey": k, "revision": r})),
                        "observed": context.observed.as_ref().map(|(k, r)| serde_json::json!({"profileKey": k, "revision": r})),
                        "state": context.state,
                        "reason": context.reason,
                        "desiredRoute": context.desired_route,
                        "observedRoute": context.observed_route,
                    });
                }
                auth_json
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
        },
    )
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
    let if_match = match require_if_match(&headers, "If-Match required for decommission") {
        Ok(v) => Some(v),
        Err(response) => return response,
    };
    let idempotency_key = match idempotency_header(&headers) {
        Ok(v) => v,
        Err(response) => return response,
    };
    registry_response(
        state
            .fleets
            .fleet_delete(&actor, &fleet_key, if_match, idempotency_key)
            .await,
        |accepted| accepted_response(&accepted),
    )
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
    change_response(state.fleets.fleet_change_get(&actor, &change_id).await)
}

/// R10-05: the fleet LIST route — every active (non-tombstoned) fleet
/// with its desired revision and incarnation.
pub(crate) async fn fleet_list(
    State(state): State<AppState>,
    auth: crate::oidc::Authenticated,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let actor = auth.actor;
    if let Err(response) = require_scope(&actor, Scope::FleetRead) {
        return response;
    }
    let (page, fetch) = match RequestPage::parse("fleets", query) {
        Ok(page) => page,
        Err(response) => return response,
    };
    match state.fleets.fleet_list(&actor, fetch).await {
        Ok(mut fleets) => {
            let next_cursor = page.finish(&mut fleets, |fleet| &fleet.0);
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
            let body = serde_json::json!({ "fleets": items, "next_cursor": next_cursor });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}
