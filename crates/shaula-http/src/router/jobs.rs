//! Authorized, read-only workflow and operation history.
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use shaula_core::jobs::{GenerationsQuery, JobsQuery, JobsReadError};
use shaula_core::operation_log::{InvocationQuery, LogQuery};
use shaula_core::registry::Scope;

use super::{require_scope, AppState};
use crate::oidc::Authenticated;
use crate::problem::problem;

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
