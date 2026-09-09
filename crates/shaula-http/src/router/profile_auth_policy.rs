//! Explicit policy publication never carries credential bytes in its DTO.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use shaula_core::registry::{AuthPolicyUpdate, MutationError, Scope};

use crate::problem::{mutation_problem, problem};
use crate::router::{
    accepted_response, idempotency_header, parse_if_match, require_scope, AppState,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthPolicyUpdateDto {
    base_revision: i64,
    target_policy: Vec<shaula_core::auth_policy::TargetSelector>,
}

pub(crate) async fn auth_policy_update(
    State(state): State<AppState>,
    Path(profile_key): Path<String>,
    auth: crate::oidc::Authenticated,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    for scope in [Scope::AuthRead, Scope::AuthWrite] {
        if let Err(response) = require_scope(&auth.actor, scope) {
            return response;
        }
    }
    let Ok(dto) = serde_json::from_slice::<AuthPolicyUpdateDto>(&body) else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SpecInvalid",
            "target policy update rejected by strict validation",
        )
        .into_response();
    };
    let idempotency_key = match idempotency_header(&headers) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let if_match = match parse_if_match(&headers) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if headers.contains_key("if-none-match") {
        return problem(
            StatusCode::BAD_REQUEST,
            "PreconditionInvalid",
            "If-None-Match is not supported for target policy updates",
        )
        .into_response();
    }
    match state
        .profiles
        .auth_policy_update(
            &auth.actor,
            &profile_key,
            AuthPolicyUpdate {
                base_revision: dto.base_revision,
                target_policy: dto.target_policy,
            },
            if_match,
            idempotency_key,
        )
        .await
    {
        Ok(Ok(accepted)) => accepted_response(&accepted),
        Ok(Err(MutationError::IdentityConflict)) => problem(
            StatusCode::CONFLICT,
            "Conflict",
            "the reviewed Active base is unavailable or changed; reopen and review the profile",
        )
        .into_response(),
        Ok(Err(error)) => mutation_problem(&error).into_response(),
        Err(_) => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal",
            "target policy publication is unavailable",
        )
        .into_response(),
    }
}
