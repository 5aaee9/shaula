use super::{login, sessions, AuthError, Authenticated};
use crate::router::AppState;
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};

pub(crate) async fn guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let anonymous = request.method() == Method::GET
        && matches!(path, "/auth/oidc/login" | "/auth/oidc/callback");
    let result = if anonymous {
        Ok(None)
    } else {
        authenticate(&state, request.method(), request.uri(), request.headers())
            .await
            .map(Some)
    };
    let mut response = match result {
        Ok(identity) => {
            if let Some(identity) = identity {
                request.extensions_mut().insert(identity);
            }
            next.run(request).await
        }
        Err(AuthError::Unauthorized)
            if request.method() == Method::GET
                && login::document(path)
                && !request.headers().contains_key(header::AUTHORIZATION) =>
        {
            let target = login::return_target(request.uri().path_and_query().map(|p| p.as_str()));
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("return_to", &target)
                .finish();
            login::redirect(&format!("/auth/oidc/login?{query}"))
                .unwrap_or_else(IntoResponse::into_response)
        }
        Err(error) => error.into_response(),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn authenticate(
    state: &AppState,
    method: &Method,
    uri: &axum::http::Uri,
    headers: &axum::http::HeaderMap,
) -> Result<Authenticated, AuthError> {
    let session = sessions::cookie_value(headers, sessions::SESSION)?;
    if headers.contains_key(header::AUTHORIZATION) {
        if session.is_some() || headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
            return Err(AuthError::Unauthorized);
        }
        let path = uri.path();
        if !(path == "/api" || path.starts_with("/api/") || matches!(path, "/livez" | "/readyz")) {
            return Err(AuthError::Unauthorized);
        }
        let raw = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthError::Unauthorized)?;
        let (scheme, token) = raw.split_once(' ').ok_or(AuthError::Unauthorized)?;
        if !scheme.eq_ignore_ascii_case("bearer") || token.contains(char::is_whitespace) {
            return Err(AuthError::Unauthorized);
        }
        let claims = state.oidc.claims(token, true).await?;
        return state.oidc.identity(&claims, true);
    }
    let session = session.ok_or(AuthError::Unauthorized)?;
    let identity = state.oidc.sessions.lock().await.get(&session)?;
    if !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        let csrf = headers.get("x-csrf-token").and_then(|v| v.to_str().ok());
        if headers.get_all(header::ORIGIN).iter().count() != 1
            || headers.get_all("x-csrf-token").iter().count() != 1
            || origin != Some(&state.oidc.config.origin)
            || !csrf
                .zip(identity.csrf.as_deref())
                .is_some_and(|(a, b)| sessions::equal(a, b))
        {
            return Err(AuthError::Csrf);
        }
    }
    Ok(identity)
}
