//! Route construction for `/api/v1`. Handlers enforce the verified OIDC actor
//! context, authorization scopes, conditional writes and idempotency
//! headers before delegating to registry ports.

pub mod fleet_routes;
pub mod profile_auth_reads;
pub mod profile_reads;
pub mod profile_routes;

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use shaula_core::registry::{
    Actor, FleetRegistryPort, HealthPort, MutationAccepted, ProfileRegistryPort, Scope,
};

use crate::oidc::{Authenticated, Oidc};
use crate::problem::problem;

/// Shared handler state.
#[derive(Clone)]
pub struct AppState {
    pub fleets: Arc<dyn FleetRegistryPort>,
    pub profiles: Arc<dyn ProfileRegistryPort>,
    pub health: Arc<dyn HealthPort>,
    /// Required initialized OIDC verifier and session boundary.
    pub oidc: Arc<Oidc>,
    /// Maximum accepted request body bytes for ARTIFACT uploads.
    pub body_limit: usize,
    /// Maximum accepted request body bytes for MANAGEMENT endpoints
    /// (R10-06: enforced by the body-limit layer, not by handlers).
    pub request_body_limit: usize,
    /// Content-addressed artifact publisher (digest-addressed uploads).
    pub artifact_publisher: Arc<dyn ArtifactPublisher>,
}

/// Artifact upload seam; `shaula-template`'s store backs the production
/// implementation.
pub trait ArtifactPublisher: Send + Sync {
    fn publish(&self, bytes: &[u8], declared_digest: &str) -> shaula_core::error::CoreResult<u64>;
}

pub(crate) fn require_scope(actor: &Actor, scope: Scope) -> Result<(), Response> {
    if actor.has(scope) {
        Ok(())
    } else {
        Err(problem(
            StatusCode::FORBIDDEN,
            "ScopeDenied",
            format!("required scope {}", scope.as_str()),
        )
        .into_response())
    }
}

pub(crate) fn idempotency_header(headers: &HeaderMap) -> Result<Option<String>, Response> {
    match headers.get("idempotency-key").and_then(|v| v.to_str().ok()) {
        None => Ok(None),
        Some(key) if !key.is_empty() && key.len() <= 128 => Ok(Some(key.to_string())),
        Some(_) => Err(problem(
            StatusCode::BAD_REQUEST,
            "IdempotencyKeyInvalid",
            "idempotency key out of bounds",
        )
        .into_response()),
    }
}

pub(crate) fn parse_if_match(headers: &HeaderMap) -> Result<Option<(String, i64)>, Response> {
    let Some(raw) = headers.get("if-match").and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    // Strong ETag form: "incarnation:revision"
    let raw = raw.trim().trim_matches('"');
    let Some((incarnation, revision)) = raw.rsplit_once(':') else {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "EtagInvalid",
            "If-Match ETag malformed",
        )
        .into_response());
    };
    match revision.parse::<i64>() {
        Ok(revision) => Ok(Some((incarnation.to_string(), revision))),
        Err(_) => Err(problem(
            StatusCode::BAD_REQUEST,
            "EtagInvalid",
            "If-Match revision malformed",
        )
        .into_response()),
    }
}

pub(crate) fn if_none_match_star(headers: &HeaderMap) -> bool {
    headers
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "*")
        .unwrap_or(false)
}

pub(crate) fn accepted_response(accepted: &MutationAccepted) -> Response {
    if accepted.no_op {
        return (
            StatusCode::OK,
            [("etag", format!("\"{}\"", accepted.etag))],
            Json(serde_json::json!({
                "changeId": accepted.change.id,
                "state": accepted.change.state,
                "revision": accepted.change.revision,
                "noOp": true,
            })),
        )
            .into_response();
    }
    (
        StatusCode::ACCEPTED,
        [("etag", format!("\"{}\"", accepted.etag))],
        Json(serde_json::json!({
            "changeId": accepted.change.id,
            "state": accepted.change.state,
            "revision": accepted.change.revision,
        })),
    )
        .into_response()
}

async fn health_live(State(state): State<AppState>) -> StatusCode {
    if state.health.live().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn health_ready(State(state): State<AppState>) -> Response {
    if state.oidc.ready().await && state.health.ready().await {
        (StatusCode::OK, Json(serde_json::json!({"ready": true}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"ready": false})),
        )
            .into_response()
    }
}

async fn session(auth: Authenticated) -> Response {
    let mut response = Json(serde_json::json!({
        "name": auth.name,
        "scopes": auth.actor.scopes.iter().map(|scope| scope.as_str()).collect::<Vec<_>>(),
    }))
    .into_response();
    if let Some(csrf) = auth
        .csrf
        .and_then(|s| s.parse::<axum::http::HeaderValue>().ok())
    {
        response.headers_mut().insert("x-csrf-token", csrf);
    }
    response
}

/// Builds the full v1 router. R10-06: the configured body limits are
/// ENFORCED at the HTTP layer — the artifact route gets the artifact
/// limit, every other route the smaller management limit — instead of
/// buffering unbounded bodies before a handler-side check.
pub fn build_router(state: AppState) -> Router {
    use axum::extract::DefaultBodyLimit;
    Router::new()
        .route("/api/v1/session", get(session))
        .route("/auth/oidc/login", get(crate::oidc::login))
        .route("/auth/oidc/callback", get(crate::oidc::callback))
        .route("/auth/oidc/logout", axum::routing::post(crate::oidc::logout))
        .route("/livez", get(health_live))
        .route("/readyz", get(health_ready))
        .route("/api/v1/profile-changes/{changeId}", get(profile_reads::profile_change_get))
        .route(
            "/api/v1/fleets/{fleetKey}",
            put(fleet_routes::fleet_put)
                .get(fleet_routes::fleet_get)
                .delete(fleet_routes::fleet_delete),
        )
        .route(
            "/api/v1/fleets/{fleetKey}/status",
            get(fleet_routes::fleet_status),
        )
        .route(
            "/api/v1/fleet-changes/{changeId}",
            get(fleet_routes::fleet_change_get),
        )
        .route(
            "/api/v1/fleets",
            get(fleet_routes::fleet_list),
        )
        .route(
            "/api/v1/template-artifacts/{digest}",
            put(profile_routes::template_artifact_put).layer(DefaultBodyLimit::max(
                state.body_limit,
            )),
        )
        .route(
            "/api/v1/template-profiles",
            get(profile_reads::template_profile_list),
        )
        .route(
            "/api/v1/template-profiles/{profileKey}/revisions/{revision}",
            get(profile_reads::template_revision_get),
        )
        .route(
            "/api/v1/template-profiles/{profileKey}/revisions/{revision}/attestations/{attestationKey}",
            put(profile_routes::template_attestation_put)
                .get(profile_reads::template_attestation_get),
        )
        .route(
            "/api/v1/github-auth-profiles/{profileKey}/status",
            get(profile_reads::auth_profile_status),
        )
        .route(
            "/api/v1/github-auth-profiles/{profileKey}/impact",
            get(profile_auth_reads::auth_profile_impact),
        )
        .route(
            "/api/v1/github-auth-profiles/{profileKey}/revisions/{revision}",
            get(profile_reads::auth_revision_get),
        )
        .route(
            "/api/v1/template-profiles/{profileKey}",
            put(profile_routes::template_profile_put)
                .get(profile_routes::template_profile_get)
                .delete(profile_reads::template_profile_delete),
        )
        .route(
            "/api/v1/github-auth-profiles/{profileKey}",
            put(profile_routes::auth_profile_put).get(profile_auth_reads::auth_profile_get)
                .delete(profile_reads::auth_profile_delete),
        )
        .layer(DefaultBodyLimit::max(state.request_body_limit))
        .fallback(crate::web::serve)
        .layer(axum::middleware::from_fn_with_state(state.clone(), crate::oidc::guard))
        .with_state(state)
}
