use axum::{
    body::to_bytes,
    extract::{Path, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use shaula_core::state_backend::{
    LockId, LockInfo, StateDocument, StateError, StateResult, MAX_LOCK_BYTES, MAX_STATE_BYTES,
};

use super::{auth, App, REQUEST_TIMEOUT};

pub(super) async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(super) async fn unknown() -> Response {
    failure(StateError::Unauthorized)
}

pub(super) async fn handle(
    State(app): State<App>,
    Path(generation): Path<String>,
    request: Request,
) -> Response {
    let Ok(_permit) = app.permits.try_acquire() else {
        return failure(StateError::Unavailable);
    };
    match tokio::time::timeout(REQUEST_TIMEOUT, dispatch(&app, &generation, request)).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => failure(error),
        Err(_) => (StatusCode::REQUEST_TIMEOUT, "backend request timed out").into_response(),
    }
}

async fn dispatch(app: &App, generation: &str, request: Request) -> StateResult<Response> {
    let access = auth::access(request.headers(), generation)?;
    // No Bytes/Json extractor may precede this call. In particular a bad
    // credential with an oversized or never-ending body must be rejected now.
    app.backend.authenticate(&access).await?;
    let method = request.method().as_str();
    let query = request.uri().query();
    if !matches!(method, "GET" | "POST" | "LOCK" | "UNLOCK") {
        return Ok((
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, POST, LOCK, UNLOCK")],
        )
            .into_response());
    }
    if method != "POST" && query.is_some() {
        return Err(StateError::Invalid);
    }
    match method {
        "GET" => match app.backend.read(&access).await? {
            None => Ok(StatusCode::NOT_FOUND.into_response()),
            Some(snapshot) => Ok(json_bytes(snapshot.document.bytes().to_vec())),
        },
        "POST" => {
            let id = update_id(query)?;
            let body = to_bytes(request.into_body(), MAX_STATE_BYTES)
                .await
                .map_err(|_| StateError::TooLarge)?;
            let document = StateDocument::parse(body.to_vec())?;
            app.backend.write(&access, &id, document).await?;
            Ok(StatusCode::OK.into_response())
        }
        "LOCK" | "UNLOCK" => {
            let locking = method == "LOCK";
            let body = to_bytes(request.into_body(), MAX_LOCK_BYTES)
                .await
                .map_err(|_| StateError::TooLarge)?;
            let info = LockInfo::parse(&body)?;
            if locking {
                app.backend.lock(&access, info).await?;
            } else {
                app.backend.unlock(&access, info.id()).await?;
            }
            Ok(StatusCode::OK.into_response())
        }
        _ => Err(StateError::Invalid),
    }
}

fn update_id(query: Option<&str>) -> StateResult<LockId> {
    let query = query.ok_or(StateError::Invalid)?;
    if query.len() > 1024 {
        return Err(StateError::Invalid);
    }
    let mut pairs = url::form_urlencoded::parse(query.as_bytes());
    let (key, value) = pairs.next().ok_or(StateError::Invalid)?;
    if key != "ID" || pairs.next().is_some() {
        return Err(StateError::Invalid);
    }
    LockId::parse(value.into_owned())
}

fn json_bytes(bytes: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
}

fn failure(error: StateError) -> Response {
    let status = match error {
        StateError::Unauthorized => {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Basic realm=\"shaula-state\"")],
                "state access denied",
            )
                .into_response();
        }
        StateError::Locked(ref lock) => {
            return match lock.to_bytes() {
                Ok(bytes) => (StatusCode::LOCKED, json_bytes(bytes)).into_response(),
                Err(_) => failure(StateError::Unavailable),
            };
        }
        StateError::Invalid => StatusCode::BAD_REQUEST,
        StateError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        StateError::Conflict | StateError::Sealed => StatusCode::CONFLICT,
        StateError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (status, error.to_string()).into_response()
}
