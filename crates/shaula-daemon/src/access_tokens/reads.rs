use super::PersonalTokens;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use shaula_core::access_tokens::*;

impl PersonalTokens {
    pub(super) fn metadata(&self, r: &TokenRecord) -> TokenMetadata {
        let now = self.clock.now_unix_ms();
        let state = if r.revoked_at_ms.is_some() {
            "revoked"
        } else if r.expires_at_ms <= now {
            "expired"
        } else if !self.active(r, now) {
            "disabled"
        } else {
            "active"
        };
        TokenMetadata {
            id: r.id.clone(),
            name: r.name.clone(),
            scopes: r.scopes.clone(),
            effective_scopes: if state == "active" {
                self.effective(r)
                    .iter()
                    .map(|s| s.as_str().to_owned())
                    .collect()
            } else {
                vec![]
            },
            created_at: timestamp(r.created_at_ms),
            expires_at: timestamp(r.expires_at_ms),
            last_used_at: r.last_used_at_ms.map(timestamp),
            revoked_at: r.revoked_at_ms.map(timestamp),
            state: state.into(),
            revision: r.revision,
        }
    }

    pub(super) async fn page(&self, owner: &str, query: TokenQuery) -> TokenResult<TokenPage> {
        let state = query.state.as_deref().unwrap_or("all");
        let limit = query.limit.unwrap_or(50);
        if !(1..=100).contains(&limit)
            || !["all", "active", "expired", "revoked", "disabled"].contains(&state)
        {
            return Err(TokenError::Invalid);
        }
        let mut before = match query.cursor {
            None => None,
            Some(cursor) => {
                if cursor.len() > 4096 {
                    return Err(TokenError::Invalid);
                }
                let bytes = URL_SAFE_NO_PAD
                    .decode(cursor)
                    .map_err(|_| TokenError::Invalid)?;
                let (v, p, s, at, id): (u8, String, String, i64, String) =
                    serde_json::from_slice(&bytes).map_err(|_| TokenError::Invalid)?;
                if v != 1 || p != owner || s != state {
                    return Err(TokenError::Invalid);
                }
                Some((at, id))
            }
        };
        let mut items = Vec::new();
        let mut last_match = None;
        loop {
            let rows = self.store.list(owner, before.clone(), 100).await?;
            let exhausted = rows.len() < 100;
            for row in rows {
                before = Some((row.created_at_ms, row.id.clone()));
                let metadata = self.metadata(&row);
                if state != "all" && metadata.state != state {
                    continue;
                }
                if items.len() == limit {
                    let (at, id) = last_match.ok_or(TokenError::Unavailable)?;
                    let cursor = serde_json::to_vec(&(1, owner, state, at, id))
                        .map_err(|_| TokenError::Unavailable)?;
                    return Ok(TokenPage {
                        items,
                        next_cursor: Some(URL_SAFE_NO_PAD.encode(cursor)),
                    });
                }
                last_match = before.clone();
                items.push(metadata);
            }
            if exhausted {
                return Ok(TokenPage {
                    items,
                    next_cursor: None,
                });
            }
        }
    }
}

fn timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}
