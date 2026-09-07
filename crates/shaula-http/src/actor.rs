//! Trusted reverse-proxy actor-context validation. Every management
//! request — proxied or direct loopback — must carry the selected trusted
//! actor assertion and backend authentication; loopback never synthesizes
//! an actor and missing/invalid context is rejected.

use axum::http::HeaderMap;
use shaula_core::registry::{Actor, Scope};

/// Header carrying the authenticated actor name injected by the trusted
/// proxy. Exact wire format is a pending contract choice; the validation
/// shape is fixed here.
pub const ACTOR_HEADER: &str = "x-shaula-actor";
pub const SCOPES_HEADER: &str = "x-shaula-scopes";
pub const BACKEND_AUTH_HEADER: &str = "x-shaula-backend-auth";

/// Caller-supplied identity headers that a proxy must strip before
/// injecting its own context. Their presence on a request is a protocol
/// violation.
pub const STRIPPED_IDENTITY_HEADERS: &[&str] =
    &["x-forwarded-user", "x-remote-user", "x-shaula-identity"];

/// Validates the trusted actor context for one request.
pub fn validate_actor(headers: &HeaderMap, backend_token: &str) -> Result<Actor, ActorError> {
    for stripped in STRIPPED_IDENTITY_HEADERS {
        if headers.contains_key(*stripped) {
            return Err(ActorError::ForgedIdentity);
        }
    }
    let provided_backend = headers
        .get(BACKEND_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or(ActorError::Missing)?;
    if !constant_time_eq(provided_backend, backend_token) {
        return Err(ActorError::Invalid);
    }
    let name = headers
        .get(ACTOR_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or(ActorError::Missing)?;
    let scopes = headers
        .get(SCOPES_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter_map(parse_scope)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Actor {
        name: name.to_string(),
        scopes,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorError {
    Missing,
    Invalid,
    ForgedIdentity,
}

fn parse_scope(raw: &str) -> Option<Scope> {
    const ALL: &[Scope] = &[
        Scope::FleetRead,
        Scope::FleetWrite,
        Scope::FleetRetire,
        Scope::TemplateRead,
        Scope::TemplatePublish,
        Scope::TemplateAttest,
        Scope::TemplateRetire,
        Scope::AuthRead,
        Scope::AuthWrite,
        Scope::AuthRetire,
    ];
    ALL.iter().copied().find(|s| s.as_str() == raw)
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(with: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (key, value) in with {
            map.insert(
                axum::http::HeaderName::from_lowercase(key.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    const TOKEN: &str = "backend-secret";

    #[test]
    fn valid_context_produces_actor() {
        let headers = headers(&[
            (BACKEND_AUTH_HEADER, TOKEN),
            (ACTOR_HEADER, "ci-bot"),
            (SCOPES_HEADER, "fleet.read, fleet.write"),
        ]);
        let actor = validate_actor(&headers, TOKEN).unwrap();
        assert_eq!(actor.name, "ci-bot");
        assert!(actor.has(Scope::FleetRead));
        assert!(actor.has(Scope::FleetWrite));
        assert!(!actor.has(Scope::TemplatePublish));
    }

    #[test]
    fn missing_or_invalid_backend_context_rejected() {
        let h = headers(&[(ACTOR_HEADER, "ci-bot")]);
        assert_eq!(validate_actor(&h, TOKEN), Err(ActorError::Missing));

        let h = headers(&[(BACKEND_AUTH_HEADER, "wrong"), (ACTOR_HEADER, "ci-bot")]);
        assert_eq!(validate_actor(&h, TOKEN), Err(ActorError::Invalid));
    }

    #[test]
    fn direct_loopback_never_synthesizes_actor() {
        // A request with no headers at all is rejected even from loopback.
        let headers = HeaderMap::new();
        assert_eq!(validate_actor(&headers, TOKEN), Err(ActorError::Missing));
    }

    #[test]
    fn caller_supplied_identity_header_is_forgery() {
        let headers = headers(&[
            (BACKEND_AUTH_HEADER, TOKEN),
            (ACTOR_HEADER, "attacker"),
            ("x-forwarded-user", "attacker"),
        ]);
        assert_eq!(
            validate_actor(&headers, TOKEN),
            Err(ActorError::ForgedIdentity)
        );
    }

    #[test]
    fn unknown_scopes_ignored_not_error() {
        let headers = headers(&[
            (BACKEND_AUTH_HEADER, TOKEN),
            (ACTOR_HEADER, "a"),
            (SCOPES_HEADER, "fleet.read,not-a-scope"),
        ]);
        let actor = validate_actor(&headers, TOKEN).unwrap();
        assert!(actor.has(Scope::FleetRead));
        assert_eq!(actor.scopes.len(), 1);
    }
}
