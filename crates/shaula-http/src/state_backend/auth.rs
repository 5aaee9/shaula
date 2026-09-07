use axum::http::{header::AUTHORIZATION, HeaderMap};
use base64::{engine::general_purpose::STANDARD, Engine};
use shaula_core::{
    secret::SecretString,
    state_backend::{StateAccess, StateCapability, StateError, StateResult},
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub(super) fn access(headers: &HeaderMap, generation: &str) -> StateResult<StateAccess> {
    let generation_id = Uuid::parse_str(generation).map_err(|_| StateError::Unauthorized)?;
    if generation_id.to_string() != generation {
        return Err(StateError::Unauthorized);
    }
    let mut headers = headers.get_all(AUTHORIZATION).iter();
    let header = headers.next().ok_or(StateError::Unauthorized)?;
    if headers.next().is_some() || header.len() > 512 {
        return Err(StateError::Unauthorized);
    }
    let (scheme, encoded) = header
        .to_str()
        .ok()
        .and_then(|h| h.split_once(' '))
        .ok_or(StateError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return Err(StateError::Unauthorized);
    }
    let decoded = Zeroizing::new(
        STANDARD
            .decode(encoded)
            .map_err(|_| StateError::Unauthorized)?,
    );
    let (username, password) = std::str::from_utf8(&decoded)
        .ok()
        .and_then(|s| s.split_once(':'))
        .ok_or(StateError::Unauthorized)?;
    if username != "shaula-state" {
        return Err(StateError::Unauthorized);
    }
    let capability = StateCapability::parse(SecretString::new(password))?;
    Ok(StateAccess {
        generation_id,
        capability,
    })
}
