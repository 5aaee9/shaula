//! Authorized, read-only workflow and operation history.
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use shaula_core::jobs::{GenerationsQuery, JobsQuery, JobsReadError};
use shaula_core::operation_log::{InvocationQuery, LogQuery};
use shaula_core::registry::Scope;

use super::{idempotency_header, require_scope, AppState};
use crate::oidc::Authenticated;
use crate::problem::{mutation_problem, problem};

fn unavailable() -> Response {
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "HistoryUnavailable",
        "History is temporarily unavailable",
    )
    .into_response()
}

fn missing() -> Response {
    problem(
        StatusCode::NOT_FOUND,
        "HistoryNotFound",
        "History record was not found",
    )
    .into_response()
}

fn read_error(error: JobsReadError) -> Response {
    match error {
        JobsReadError::InvalidQuery(reason) => {
            problem(StatusCode::BAD_REQUEST, "HistoryQueryInvalid", reason).into_response()
        }
        JobsReadError::Unavailable => unavailable(),
    }
}

fn private_json<T: serde::Serialize>(body: T) -> Response {
    (
        [
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Json(body),
    )
        .into_response()
}

pub(super) async fn list(
    State(state): State<AppState>,
    auth: Authenticated,
    query: Result<Query<JobsQuery>, QueryRejection>,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::FleetRead) {
        return response;
    }
    let Ok(Query(query)) = query else {
        return read_error(JobsReadError::InvalidQuery("invalid query parameters"));
    };
    let Some(jobs) = state.jobs else {
        return unavailable();
    };
    match jobs.list_jobs(query).await {
        Ok(page) => private_json(page),
        Err(error) => read_error(error),
    }
}

pub(super) async fn detail(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::FleetRead) {
        return response;
    }
    let Some(jobs) = state.jobs else {
        return unavailable();
    };
    match jobs.get_job(&id).await {
        Ok(Some(detail)) => private_json(detail),
        Ok(None) => missing(),
        Err(error) => read_error(error),
    }
}

pub(super) async fn generations(
    State(state): State<AppState>,
    auth: Authenticated,
    query: Result<Query<GenerationsQuery>, QueryRejection>,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::FleetRead) {
        return response;
    }
    let Ok(Query(query)) = query else {
        return read_error(JobsReadError::InvalidQuery("invalid query parameters"));
    };
    let Some(jobs) = state.jobs else {
        return unavailable();
    };
    match jobs.list_generations(query).await {
        Ok(page) => private_json(page),
        Err(error) => read_error(error),
    }
}

pub(super) async fn generation(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::FleetRead) {
        return response;
    }
    let Some(jobs) = state.jobs else {
        return unavailable();
    };
    match jobs.get_generation(&id).await {
        Ok(Some(detail)) => private_json(detail),
        Ok(None) => missing(),
        Err(error) => read_error(error),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct GenerationFinalizeBody {
    /// Operator's out-of-band verification statement (spec 0028 §5.3).
    reason: String,
}

/// `POST /api/v1/generations/{id}/finalize` — operator disposition of a
/// Quarantined generation (spec 0028): ledger-only transition to
/// Destroyed once the operator verified the external resources are gone.
/// No remote effect runs; the request is the attestation.
pub(super) async fn generation_finalize(
    State(state): State<AppState>,
    auth: Authenticated,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<GenerationFinalizeBody>,
) -> Response {
    // fleet.retire: the same privilege as decommission — never lower.
    if let Err(response) = require_scope(&auth.actor, Scope::FleetRetire) {
        return response;
    }
    let idempotency = match idempotency_header(&headers) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .fleets
        .generation_finalize(&auth.actor, &id, &body.reason, idempotency)
        .await
    {
        Ok(Ok(accepted)) => (
            StatusCode::ACCEPTED,
            [(
                header::CONTENT_LOCATION,
                format!("/api/v1/changes/{}", accepted.change.id),
            )],
            Json(accepted),
        )
            .into_response(),
        Ok(Err(mutation)) => mutation_problem(&mutation).into_response(),
        Err(e) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Internal", e.summary).into_response(),
    }
}

pub(super) async fn invocations(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
    query: Result<Query<InvocationQuery>, QueryRejection>,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::FleetRead) {
        return response;
    }
    let Ok(Query(query)) = query else {
        return read_error(JobsReadError::InvalidQuery("invalid query parameters"));
    };
    let (Some(jobs), Some(logs)) = (state.jobs, state.logs) else {
        return unavailable();
    };
    match jobs.get_generation(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return missing(),
        Err(error) => return read_error(error),
    }
    match logs.list_invocations_page(&id, query).await {
        Ok(page) => private_json(page),
        Err(error) if error.code == shaula_core::ReasonCode::SpecInvalid => read_error(
            JobsReadError::InvalidQuery("invalid invocation cursor or limit"),
        ),
        Err(_) => unavailable(),
    }
}

pub(super) async fn logs(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
    query: Result<Query<LogQuery>, QueryRejection>,
) -> Response {
    for scope in [Scope::FleetRead, Scope::LogsRead] {
        if let Err(response) = require_scope(&auth.actor, scope) {
            return response;
        }
    }
    let Ok(Query(query)) = query else {
        return read_error(JobsReadError::InvalidQuery("invalid query parameters"));
    };
    if query
        .limit_bytes
        .is_some_and(|limit| limit == 0 || limit > 1024 * 1024)
        || query
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > 2048)
        || query
            .phase
            .as_deref()
            .is_some_and(|phase| !["init", "plan", "apply"].contains(&phase))
        || query
            .stream
            .as_deref()
            .is_some_and(|stream| !["stdout", "stderr"].contains(&stream))
    {
        return problem(
            StatusCode::BAD_REQUEST,
            "LogQueryInvalid",
            "Invalid log page filter or limit",
        )
        .into_response();
    }
    let Some(logs) = state.logs else {
        return unavailable();
    };
    match logs.read_page(&id, query).await {
        Ok(page) => private_json(page),
        Err(error) if error.code == shaula_core::ReasonCode::TargetHiddenOrNotFound => missing(),
        Err(error) if error.code == shaula_core::ReasonCode::SpecInvalid => problem(
            StatusCode::BAD_REQUEST,
            "LogQueryInvalid",
            "Invalid log cursor or query",
        )
        .into_response(),
        Err(_) => unavailable(),
    }
}
