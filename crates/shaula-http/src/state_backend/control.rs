use axum::{
    body::to_bytes,
    extract::{Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use shaula_core::{
    secret::SecretString,
    state_backend::{StateError, StateResult, MAX_STATE_BYTES},
    worker::{
        wire::{ControlRequest, LogRequest, WorkerControl},
        ControlCapability, MAX_CONTROL_BYTES,
    },
};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Clone)]
struct ControlApp {
    service: Arc<dyn WorkerControl>,
    permits: Arc<Semaphore>,
    log_permits: Arc<Semaphore>,
}

pub(super) fn router(service: Arc<dyn WorkerControl>) -> Router {
    Router::new()
        .route(
            "/internal/v1/generations/{generation}/control",
            any(control),
        )
        .route(
            "/internal/v1/generations/{generation}/materials/{material}",
            any(material),
        )
        .route("/internal/v1/generations/{generation}/logs", any(logs))
        .with_state(ControlApp {
            service,
            permits: Arc::new(Semaphore::new(16)),
            log_permits: Arc::new(Semaphore::new(8)),
        })
        .layer(middleware::from_fn(super::handler::no_store))
}

async fn logs(
    State(app): State<ControlApp>,
    Path(generation): Path<String>,
    request: Request,
) -> Response {
    let Ok(_permit) = app.log_permits.try_acquire() else {
        return failure(StateError::Unavailable);
    };
    respond(
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (id, token) = credential(request.headers(), &generation)?;
            app.service.authenticate(id, &token).await?;
            if request.method() != "POST" || request.uri().query().is_some() {
                return Err(StateError::Invalid);
            }
            let body = to_bytes(request.into_body(), 512 * 1024)
                .await
                .map_err(|_| StateError::TooLarge)?;
            let message: LogRequest =
                serde_json::from_slice(&body).map_err(|_| StateError::Invalid)?;
            let result = app.service.logs(id, &token, message).await?;
            let bytes = serde_json::to_vec(&result).map_err(|_| StateError::Unavailable)?;
            if bytes.len() > 512 * 1024 {
                return Err(StateError::TooLarge);
            }
            Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
        })
        .await,
    )
}

fn credential(headers: &HeaderMap, generation: &str) -> StateResult<(Uuid, ControlCapability)> {
    let id = Uuid::parse_str(generation).map_err(|_| StateError::Unauthorized)?;
    if id.to_string() != generation {
        return Err(StateError::Unauthorized);
    }
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().ok_or(StateError::Unauthorized)?;
    if values.next().is_some() || value.len() > 512 {
        return Err(StateError::Unauthorized);
    }
    let (scheme, token) = value
        .to_str()
        .ok()
        .and_then(|text| text.split_once(' '))
        .ok_or(StateError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(StateError::Unauthorized);
    }
    Ok((id, ControlCapability::parse(SecretString::new(token))?))
}

async fn control(
    State(app): State<ControlApp>,
    Path(generation): Path<String>,
    request: Request,
) -> Response {
    let Ok(_permit) = app.permits.try_acquire() else {
        return failure(StateError::Unavailable);
    };
    respond(
        tokio::time::timeout(super::REQUEST_TIMEOUT, async {
            let (id, token) = credential(request.headers(), &generation)?;
            app.service.authenticate(id, &token).await?;
            if request.method() != "POST" || request.uri().query().is_some() {
                return Err(StateError::Invalid);
            }
            let body = to_bytes(request.into_body(), MAX_CONTROL_BYTES)
                .await
                .map_err(|_| StateError::TooLarge)?;
            let message: ControlRequest =
                serde_json::from_slice(&body).map_err(|_| StateError::Invalid)?;
            if message.claim.generation_id != id {
                return Err(StateError::Unauthorized);
            }
            let response = app.service.call(&token, message).await?;
            let bytes = serde_json::to_vec(&response).map_err(|_| StateError::Unavailable)?;
            if bytes.len() > MAX_CONTROL_BYTES {
                return Err(StateError::TooLarge);
            }
            Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
        })
        .await,
    )
}

async fn material(
    State(app): State<ControlApp>,
    Path((generation, material)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let Ok(_permit) = app.permits.try_acquire() else {
        return failure(StateError::Unavailable);
    };
    respond(
        tokio::time::timeout(super::REQUEST_TIMEOUT, async {
            let (id, token) = credential(&parts.headers, &generation)?;
            app.service.authenticate(id, &token).await?;
            if parts.method != "GET" || parts.uri.query().is_some() {
                return Err(StateError::Invalid);
            }
            let material = Uuid::parse_str(&material).map_err(|_| StateError::Invalid)?;
            let bytes = app.service.material(id, &token, material).await?;
            if bytes.len() > MAX_STATE_BYTES {
                return Err(StateError::TooLarge);
            }
            Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
        })
        .await,
    )
}

fn respond(result: Result<StateResult<Response>, tokio::time::error::Elapsed>) -> Response {
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => failure(error),
        Err(_) => failure(StateError::Unavailable),
    }
}

fn failure(error: StateError) -> Response {
    let status = match error {
        StateError::Unauthorized => StatusCode::UNAUTHORIZED,
        StateError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        StateError::Invalid => StatusCode::BAD_REQUEST,
        StateError::Conflict | StateError::Locked(_) | StateError::Sealed => StatusCode::CONFLICT,
        StateError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (status, "worker request rejected").into_response()
}
