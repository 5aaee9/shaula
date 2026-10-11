use super::*;
use futures::StreamExt;

/// Outcome of reading a response body under a byte limit.
pub(crate) enum BodyRead {
    Complete(Vec<u8>),
    /// The stream failed; carries the bytes received before the failure.
    Interrupted(Vec<u8>),
    /// The body would exceed the limit; nothing past it is buffered.
    TooLarge,
}

/// Streams `response` into memory, stopping at the first stream error or
/// at the first chunk that would push the body past `limit` bytes.
pub(crate) async fn read_bounded(response: reqwest::Response, limit: usize) -> BodyRead {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return BodyRead::Interrupted(bytes);
        };
        if bytes.len() + chunk.len() > limit {
            return BodyRead::TooLarge;
        }
        bytes.extend_from_slice(&chunk);
    }
    BodyRead::Complete(bytes)
}

pub(crate) async fn http_error(response: reqwest::Response) -> Error {
    let status = response.status().as_u16();
    let retry_after_seconds = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| uuid::Uuid::parse_str(v).is_ok())
        .map(str::to_owned);
    let fallback = match status {
        401 => "AuthenticationRequired",
        403 => "ScopeDenied",
        409 => "Conflict",
        412 => "PreconditionFailed",
        428 => "PreconditionRequired",
        404 => "NotFound",
        429 => "RateLimited",
        _ => "RequestFailed",
    };
    let json = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';').next().is_some_and(|t| {
                matches!(t.trim(), "application/json" | "application/problem+json")
            })
        });
    let mut code = fallback.to_owned();
    if json {
        // A broken stream keeps its partial bytes; an oversized one is dropped.
        let bytes = match read_bounded(response, 65_536).await {
            BodyRead::Complete(bytes) | BodyRead::Interrupted(bytes) => bytes,
            BodyRead::TooLarge => Vec::new(),
        };
        if let Ok(problem) = serde_json::from_slice::<types::ProblemDetails>(&bytes) {
            if let Some(value) = problem.code.filter(|v| {
                !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_alphanumeric())
            }) {
                code = value;
            }
        }
    }
    Error::Http {
        status,
        code,
        retry_after_seconds,
        request_id,
    }
}
