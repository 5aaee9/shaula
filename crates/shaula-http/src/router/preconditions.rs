//! Conditional-write header parsing (`If-Match` / `If-None-Match`).

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::problem::problem;

/// `(create, if_match)`: `create` is `If-None-Match: *`.
pub(crate) type PutPreconditions = (bool, Option<(String, i64)>);

/// Conditional PUT preconditions. A malformed `If-Match` is rejected
/// first; a PUT carrying neither header is `428 PreconditionRequired`.
pub(crate) fn put_preconditions(headers: &HeaderMap) -> Result<PutPreconditions, Response> {
    let create = if_none_match_star(headers);
    let if_match = parse_if_match(headers)?;
    if !create && if_match.is_none() {
        return Err(problem(
            StatusCode::PRECONDITION_REQUIRED,
            "PreconditionRequired",
            "If-None-Match: * or If-Match required",
        )
        .into_response());
    }
    Ok((create, if_match))
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
