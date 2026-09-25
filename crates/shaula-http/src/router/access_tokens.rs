use super::{require_scope, resource_version_headers, AppState};
use crate::{oidc::Authenticated, problem::problem};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use shaula_core::{access_tokens::*, registry::Scope};

pub(crate) fn error(error: TokenError) -> Response {
    let (status, code) = match error {
        TokenError::Unauthorized => (StatusCode::UNAUTHORIZED, "AuthenticationRequired"),
        TokenError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "AuthenticationUnavailable"),
        TokenError::Forbidden => (StatusCode::FORBIDDEN, "ScopeDenied"),
        TokenError::PrimaryRequired => (StatusCode::FORBIDDEN, "PrimaryAuthenticationRequired"),
        TokenError::PersonalRequired => (StatusCode::CONFLICT, "PersonalTokenRequired"),
        TokenError::NotFound => (StatusCode::NOT_FOUND, "NotFound"),
        TokenError::Invalid => (StatusCode::UNPROCESSABLE_ENTITY, "TokenRequestInvalid"),
        TokenError::NonDelegable => (StatusCode::UNPROCESSABLE_ENTITY, "NonDelegableScope"),
        TokenError::Disabled => (StatusCode::FORBIDDEN, "AccessTokensDisabled"),
        TokenError::Conflict => (StatusCode::CONFLICT, "IdempotencyConflict"),
        TokenError::Quota => (StatusCode::TOO_MANY_REQUESTS, "TokenQuotaExceeded"),
        TokenError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "TokenRateLimited"),
        TokenError::PreconditionRequired => {
            (StatusCode::PRECONDITION_REQUIRED, "PreconditionRequired")
        }
        TokenError::PreconditionFailed => (StatusCode::PRECONDITION_FAILED, "PreconditionFailed"),
    };
    let mut response = problem(status, code, error.to_string()).into_response();
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            "www-authenticate",
            axum::http::HeaderValue::from_static("Bearer"),
        );
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        response
            .headers_mut()
            .insert("retry-after", axum::http::HeaderValue::from_static("60"));
    }
    response
}

pub(super) async fn issue(
    State(state): State<AppState>,
    auth: Authenticated,
    headers: HeaderMap,
    body: Result<Json<IssueToken>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if auth.token_id.is_some() {
        return error(TokenError::PrimaryRequired);
    }
    if let Err(response) = require_scope(&auth.actor, Scope::AccessTokenWrite) {
        return response;
    }
    let Some(service) = state.access_tokens else {
        return error(TokenError::Unavailable);
    };
    let body = match body {
        Ok(Json(body)) => body,
        Err(rejection) => {
            return problem(
                rejection.status(),
                "TokenRequestInvalid",
                "invalid token request",
            )
            .into_response()
        }
    };
    let Some(key) = headers.get("idempotency-key").and_then(|v| v.to_str().ok()) else {
        return problem(
            StatusCode::BAD_REQUEST,
            "IdempotencyKeyInvalid",
            "Idempotency-Key is required",
        )
        .into_response();
    };
    if headers.get_all("idempotency-key").iter().count() != 1
        || key.is_empty()
        || key.len() > 128
        || key.chars().any(char::is_control)
    {
        return problem(
            StatusCode::BAD_REQUEST,
            "IdempotencyKeyInvalid",
            "invalid Idempotency-Key",
        )
        .into_response();
    }
    match service.issue(&auth.actor, true, body, key).await {
        Err(e) => error(e),
        Ok(issued) => {
            let location = format!("/api/v1/access-tokens/{}", issued.access_token.id);
            let status = if issued.secret.is_some() {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            // The only server serialization path allowed to hand off plaintext.
            let mut value = serde_json::json!({"access_token":issued.access_token,"secret_available":issued.secret.is_some()});
            if let Some(secret) = issued.secret {
                value["token"] = secret.expose().into();
            }
            (status, [("location", location)], Json(value)).into_response()
        }
    }
}

pub(super) async fn list(
    State(state): State<AppState>,
    auth: Authenticated,
    query: Result<Query<TokenQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if let Err(r) = require_scope(&auth.actor, Scope::AccessTokenRead) {
        return r;
    }
    let Some(service) = state.access_tokens else {
        return error(TokenError::Unavailable);
    };
    let Ok(Query(query)) = query else {
        return error(TokenError::Invalid);
    };
    match service.list(&auth.actor.name, query).await {
        Ok(page) => Json(page).into_response(),
        Err(e) => error(e),
    }
}
pub(super) async fn get(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_scope(&auth.actor, Scope::AccessTokenRead) {
        return r;
    }
    detail(&state, &auth, &id).await
}
pub(super) async fn current(State(state): State<AppState>, auth: Authenticated) -> Response {
    let Some(id) = auth.token_id.as_deref() else {
        return error(TokenError::PersonalRequired);
    };
    detail(&state, &auth, id).await
}
async fn detail(state: &AppState, auth: &Authenticated, id: &str) -> Response {
    let Some(service) = &state.access_tokens else {
        return error(TokenError::Unavailable);
    };
    match service.get(&auth.actor.name, id).await {
        Ok(m) => (
            resource_version_headers(&format!("{}:{}", m.id, m.revision)),
            Json(m),
        )
            .into_response(),
        Err(e) => error(e),
    }
}
pub(super) async fn revoke(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_scope(&auth.actor, Scope::AccessTokenRevoke) {
        return r;
    }
    let Some(service) = &state.access_tokens else {
        return error(TokenError::Unavailable);
    };
    // Hide foreign IDs before evaluating their write versions.
    if let Err(e) = service.get(&auth.actor.name, &id).await {
        return error(e);
    }
    let Some(raw) = headers.get("if-match").and_then(|v| v.to_str().ok()) else {
        return error(TokenError::PreconditionRequired);
    };
    let prefix = format!("\"{id}:");
    let revision = raw
        .strip_prefix(&prefix)
        .and_then(|v| v.strip_suffix('"'))
        .and_then(|v| v.parse::<i64>().ok());
    let Some(revision) = revision.filter(|_| headers.get_all("if-match").iter().count() == 1)
    else {
        return error(TokenError::PreconditionFailed);
    };
    match service.revoke(&auth.actor, &id, Some(revision)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(e),
    }
}
pub(super) async fn revoke_current(State(state): State<AppState>, auth: Authenticated) -> Response {
    let Some(id) = auth.token_id.as_deref() else {
        return error(TokenError::PersonalRequired);
    };
    let Some(service) = state.access_tokens else {
        return error(TokenError::Unavailable);
    };
    match service.revoke(&auth.actor, id, None).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(e),
    }
}
