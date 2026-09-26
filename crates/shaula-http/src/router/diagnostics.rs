//! Authorization and serialization only. GET never invokes a controller.
use super::{require_scope, AppState};
use crate::{oidc::Authenticated, problem::problem};
use axum::{
    extract::{Path, RawQuery, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use shaula_core::{diagnostics::SubjectKind, registry::Scope};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/fleets/{key}/diagnostics", get(fleet))
        .route("/api/v1/generations/{key}/diagnostics", get(generation))
        .route("/api/v1/jobs/{key}/diagnostics", get(job))
        .route_layer(axum::middleware::from_fn(no_store))
}
async fn no_store(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().remove(header::ETAG);
    response.headers_mut().remove("shaula-resource-version");
    response
}
macro_rules! handler {
    ($name:ident, $kind:ident) => {
        async fn $name(
            State(state): State<AppState>,
            auth: Authenticated,
            Path(key): Path<String>,
            RawQuery(query): RawQuery,
        ) -> Response {
            read(state, auth, key, query, SubjectKind::$kind).await
        }
    };
}
handler!(fleet, Fleet);
handler!(generation, Generation);
handler!(job, Job);

async fn read(
    state: AppState,
    auth: Authenticated,
    key: String,
    query: Option<String>,
    kind: SubjectKind,
) -> Response {
    if let Err(response) = require_scope(&auth.actor, Scope::FleetRead) {
        return response;
    }
    if query.is_some_and(|query| !query.is_empty()) {
        return problem(
            StatusCode::BAD_REQUEST,
            "DiagnosticsQueryInvalid",
            "diagnostics accepts no query parameters",
        )
        .into_response();
    }
    match state.fleets.diagnostics(&auth.actor, kind, &key).await {
        Ok(Some(report)) => match serde_json::to_vec(&report) {
            Ok(body) if body.len() <= 256 * 1024 => Json(report).into_response(),
            _ => problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "DiagnosticsUnavailable",
                "diagnostics response unavailable",
            )
            .into_response(),
        },
        Ok(None) => problem(
            StatusCode::NOT_FOUND,
            "DiagnosticsNotFound",
            "diagnostic subject was not found",
        )
        .into_response(),
        Err(shaula_core::diagnostics::DiagnosticsReadError::Gone) => problem(
            StatusCode::GONE,
            "DiagnosticsGone",
            "diagnostic subject has been removed",
        )
        .into_response(),
        Err(_) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "DiagnosticsUnavailable",
            "diagnostics are temporarily unavailable",
        )
        .into_response(),
    }
}
