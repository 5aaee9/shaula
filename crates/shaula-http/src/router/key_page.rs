//! Bounded keyset pagination for registry list endpoints (spec 0002 §5.3,
//! spec 0005 §4). The cursor is opaque to clients and bound to one resource
//! kind, so a Fleet cursor cannot page Template Profiles.

use axum::extract::rejection::QueryRejection;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde::Deserialize;
use shaula_core::registry::KeyPage;

use crate::problem::problem;

pub(crate) const DEFAULT_LIMIT: usize = 100;
pub(crate) const MAX_LIMIT: usize = 200;
const MAX_CURSOR: usize = 512;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListQuery {
    limit: Option<usize>,
    cursor: Option<String>,
}

/// A validated request page: `fetch` asks the store for one extra row.
pub(crate) struct RequestPage {
    kind: &'static str,
    limit: usize,
}

impl RequestPage {
    pub(crate) fn parse(
        kind: &'static str,
        query: Result<Query<ListQuery>, QueryRejection>,
    ) -> Result<(Self, KeyPage), Response> {
        let Ok(Query(query)) = query else {
            return Err(invalid());
        };
        let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
        if limit == 0 || limit > MAX_LIMIT {
            return Err(invalid());
        }
        let after = match query.cursor {
            None => None,
            Some(cursor) => Some(decode(kind, &cursor).ok_or_else(invalid)?),
        };
        let fetch = KeyPage {
            after,
            limit: Some(limit + 1),
        };
        Ok((Self { kind, limit }, fetch))
    }

    /// Truncates `items` to the page and returns the cursor for the next one.
    pub(crate) fn finish<T>(&self, items: &mut Vec<T>, key: impl Fn(&T) -> &str) -> Option<String> {
        if items.len() <= self.limit {
            return None;
        }
        items.truncate(self.limit);
        items.last().map(|item| encode(self.kind, key(item)))
    }
}

fn encode(kind: &str, key: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("v1:{kind}:{key}"))
}

fn decode(kind: &str, cursor: &str) -> Option<String> {
    if cursor.is_empty() || cursor.len() > MAX_CURSOR {
        return None;
    }
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()?;
    let text = String::from_utf8(raw).ok()?;
    let key = text
        .strip_prefix("v1:")?
        .strip_prefix(kind)?
        .strip_prefix(':')?;
    (!key.is_empty()).then(|| key.to_owned())
}

fn invalid() -> Response {
    problem(
        StatusCode::BAD_REQUEST,
        "ListQueryInvalid",
        "Invalid list cursor or limit",
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips_and_is_bound_to_its_kind() {
        let cursor = encode("fleets", "alpha");
        assert_eq!(decode("fleets", &cursor).as_deref(), Some("alpha"));
        assert_eq!(decode("template-profiles", &cursor), None);
        assert_eq!(decode("fleets", "not base64!"), None);
        assert_eq!(decode("fleets", &encode("fleets", "")), None);
    }
}
