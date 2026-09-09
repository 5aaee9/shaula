use super::Invocation;
use crate::error::{CoreError, CoreResult, ReasonCode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationsPage {
    pub items: Vec<Invocation>,
    pub next_cursor: Option<String>,
    pub latest_create: Option<Invocation>,
    pub latest_destroy: Option<Invocation>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    generation_id: String,
    started_at: i64,
    id: String,
}

fn invalid() -> CoreError {
    CoreError::new(ReasonCode::SpecInvalid, "invalid invocation page request")
}

impl InvocationQuery {
    pub fn page_limit(&self) -> CoreResult<usize> {
        let limit = self.limit.unwrap_or(50);
        if !(1..=200).contains(&limit) {
            return Err(invalid());
        }
        usize::try_from(limit).map_err(|_| invalid())
    }

    /// Opaque cursor decoding is shared by memory adapters and durable keyset queries.
    pub fn position_for(&self, generation_id: &str) -> CoreResult<Option<(i64, String)>> {
        uuid::Uuid::parse_str(generation_id).map_err(|_| invalid())?;
        let Some(cursor) = &self.cursor else {
            return Ok(None);
        };
        if cursor.len() > 1024 {
            return Err(invalid());
        }
        let bytes = hex::decode(cursor).map_err(|_| invalid())?;
        let position: Position = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if position.generation_id != generation_id {
            return Err(invalid());
        }
        uuid::Uuid::parse_str(&position.id).map_err(|_| invalid())?;
        Ok(Some((position.started_at, position.id)))
    }

    pub fn cursor_after(generation_id: &str, item: &Invocation) -> CoreResult<String> {
        serde_json::to_vec(&Position {
            generation_id: generation_id.into(),
            started_at: item.started_at,
            id: item.id.clone(),
        })
        .map(hex::encode)
        .map_err(|_| invalid())
    }

    pub(super) fn page(
        &self,
        generation_id: &str,
        mut items: Vec<Invocation>,
    ) -> CoreResult<InvocationsPage> {
        let limit = self.page_limit()?;
        let position = self.position_for(generation_id)?;
        let latest = |operation: &str| {
            items
                .iter()
                .filter(|item| item.operation == operation)
                .max_by(|a, b| {
                    (a.started_at, a.ordinal, &a.id).cmp(&(b.started_at, b.ordinal, &b.id))
                })
                .cloned()
        };
        let latest_create = latest("Create");
        let latest_destroy = latest("Destroy");
        items.sort_by(|a, b| (b.started_at, &b.id).cmp(&(a.started_at, &a.id)));
        if let Some((started_at, id)) = position {
            items.retain(|item| (item.started_at, &item.id) < (started_at, &id));
        }
        let more = items.len() > limit;
        items.truncate(limit);
        let next_cursor = if more {
            items
                .last()
                .map(|item| Self::cursor_after(generation_id, item))
                .transpose()?
        } else {
            None
        };
        Ok(InvocationsPage {
            items,
            next_cursor,
            latest_create,
            latest_destroy,
        })
    }
}
