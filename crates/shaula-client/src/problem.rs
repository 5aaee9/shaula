use super::*;
use futures::StreamExt;
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
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(Ok(chunk)) = stream.next().await {
            if bytes.len() + chunk.len() > 65_536 {
                bytes.clear();
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
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
