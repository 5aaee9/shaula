//! Embedded UI assets, served only behind the global OIDC boundary.

use axum::{
    body::Body,
    http::{header, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../web/dist/"]
struct WebAssets;

pub(crate) async fn serve(method: Method, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Never turn missing API endpoints or asset requests into successful HTML.
    if path == "api" || path.starts_with("api/") || matches!(path, "livez" | "readyz") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }
    let document = crate::oidc::document(uri.path());
    let asset_path = if document { "index.html" } else { path };
    let Some(asset) = WebAssets::get(asset_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let len = asset.data.len();
    let mime = mime_guess::from_path(asset_path).first_or_octet_stream();
    let cache = "private, no-store";
    let mut response = (
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::CACHE_CONTROL, cache.to_string()),
            (header::CONTENT_LENGTH, len.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::REFERRER_POLICY, "no-referrer".to_string()),
            (header::CONTENT_SECURITY_POLICY, "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'".to_string()),
        ],
        asset.data,
    ).into_response();
    if method == Method::HEAD {
        *response.body_mut() = Body::empty();
    }
    response
}

#[cfg(test)]
mod tests;
