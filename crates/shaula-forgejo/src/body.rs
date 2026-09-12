use futures::StreamExt;
use reqwest::Response;

use crate::client::MAX_RESPONSE_BYTES;
use crate::error::ForgejoError;

pub(crate) async fn read_body(response: Response) -> Result<Vec<u8>, ForgejoError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ForgejoError::ResponseTooLarge);
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ForgejoError::InvalidResponse(error.to_string()))?;
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
            return Err(ForgejoError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}
