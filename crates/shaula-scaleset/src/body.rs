//! Bounded response-body reads. GitHub and the Actions service are trusted
//! for protocol semantics but not for resource use: a body is streamed and
//! abandoned as soon as it exceeds its limit instead of being buffered whole.

use serde::de::DeserializeOwned;

/// Ceiling for single-object responses (tokens, identities, messages, errors).
pub(crate) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Ceiling for the unfiltered runner inventory, which lists every runner
/// registered with the Actions service (roughly 1 KiB each upstream).
pub(crate) const MAX_RUNNER_LIST_BYTES: usize = 32 * 1024 * 1024;

/// Why a body could not be read. `Display` never echoes body content.
#[derive(Debug)]
pub(crate) enum BodyError {
    TooLarge,
    Read,
    Decode,
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BodyError::TooLarge => "response body too large",
            BodyError::Read => "error reading response body",
            BodyError::Decode => "error decoding response body",
        })
    }
}

/// Reads at most `limit` bytes of `response`, failing once it is exceeded.
pub(crate) async fn read_bytes(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(BodyError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| BodyError::Read)? {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(BodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Decodes a JSON body of at most `limit` bytes.
pub(crate) async fn read_json<T: DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> Result<T, BodyError> {
    let body = read_bytes(response, limit).await?;
    serde_json::from_slice(&body).map_err(|_| BodyError::Decode)
}

/// Bounded replacements for `reqwest::Response::{json, bytes}`.
pub(crate) trait BoundedBody {
    /// `Response::json` limited to [`MAX_RESPONSE_BYTES`].
    async fn bounded_json<T: DeserializeOwned>(self) -> Result<T, BodyError>;
    /// `Response::bytes` limited to [`MAX_RESPONSE_BYTES`].
    async fn bounded_bytes(self) -> Result<Vec<u8>, BodyError>;
}

impl BoundedBody for reqwest::Response {
    async fn bounded_json<T: DeserializeOwned>(self) -> Result<T, BodyError> {
        read_json(self, MAX_RESPONSE_BYTES).await
    }

    async fn bounded_bytes(self) -> Result<Vec<u8>, BodyError> {
        read_bytes(self, MAX_RESPONSE_BYTES).await
    }
}
