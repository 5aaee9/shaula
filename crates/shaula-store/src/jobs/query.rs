//! Keyset pagination binds the cursor to the complete filter, never a row offset.
use sea_orm::sea_query::{
    Alias, BinOper, Condition, Expr, Func, Order, SelectStatement, SimpleExpr,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use shaula_core::jobs::{GenerationsQuery, JobsQuery, JobsReadError};

pub(super) struct PageQuery {
    condition: Condition,
    pub limit: usize,
    binding: String,
}

/// A column of the paged view, addressed unqualified.
fn field(column: &str) -> Expr {
    Expr::col(Alias::new(column))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    binding: String,
    created_at: i64,
    id: String,
}

impl PageQuery {
    pub fn jobs(query: &JobsQuery) -> Result<Self, JobsReadError> {
        if query.status.as_deref().is_some_and(|s| {
            ![
                "queued",
                "assigned",
                "running",
                "completed",
                "assignment_withdrawn",
                "unknown",
            ]
            .contains(&s)
        }) {
            return Err(JobsReadError::InvalidQuery("unsupported status"));
        }
        let mut filters = query.clone();
        filters.cursor = None;
        let mut page = Self::new(
            "jobs",
            &filters,
            query.limit,
            query.cursor.as_deref(),
            query.since,
            query.until,
        )?;
        page.equal("fleet_key", query.fleet_key.as_deref())?;
        page.equal("status", query.status.as_deref())?;
        page.equal("repository", query.repository.as_deref())?;
        page.contains("job_name", query.job_name.as_deref())?;
        Ok(page)
    }

    pub fn generations(query: &GenerationsQuery) -> Result<Self, JobsReadError> {
        if query
            .status
            .as_deref()
            .is_some_and(|s| shaula_core::lifecycle::GenerationState::from_str_repr(s).is_err())
        {
            return Err(JobsReadError::InvalidQuery("unsupported generation state"));
        }
        if query
            .association
            .as_deref()
            .is_some_and(|s| !["unassigned", "unverified", "verified", "ambiguous"].contains(&s))
        {
            return Err(JobsReadError::InvalidQuery("unsupported association"));
        }
        let mut filters = query.clone();
        filters.cursor = None;
        let mut page = Self::new(
            "generations",
            &filters,
            query.limit,
            query.cursor.as_deref(),
            query.since,
            query.until,
        )?;
        page.equal("fleet_key", query.fleet_key.as_deref())?;
        page.equal("state", query.status.as_deref())?;
        if query.association.as_deref() == Some("unassigned") {
            page.and(field("association_status").ne("verified"));
        } else {
            page.equal("association_status", query.association.as_deref())?;
        }
        Ok(page)
    }

    fn new<T: Serialize>(
        kind: &str,
        filters: &T,
        limit: Option<u32>,
        cursor: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Self, JobsReadError> {
        let limit = limit.unwrap_or(50);
        if !(1..=200).contains(&limit) {
            return Err(JobsReadError::InvalidQuery(
                "limit must be between 1 and 200",
            ));
        }
        if since.is_some_and(|t| t < 0)
            || until.is_some_and(|t| t < 0)
            || since.zip(until).is_some_and(|(s, u)| s > u)
        {
            return Err(JobsReadError::InvalidQuery("invalid time range"));
        }
        let json =
            serde_json::to_string(&(kind, filters)).map_err(|_| JobsReadError::Unavailable)?;
        let binding = hex::encode(sha2::Sha256::digest(json.as_bytes()));
        let mut result = Self {
            condition: Condition::all(),
            limit: limit as usize,
            binding,
        };
        if let Some(since) = since {
            result.and(field("created_at").gte(since));
        }
        if let Some(until) = until {
            result.and(field("created_at").lte(until));
        }
        if let Some(raw) = cursor {
            if raw.len() > 2048 {
                return Err(JobsReadError::InvalidQuery("invalid cursor"));
            }
            let bytes =
                hex::decode(raw).map_err(|_| JobsReadError::InvalidQuery("invalid cursor"))?;
            let cursor: Cursor = serde_json::from_slice(&bytes)
                .map_err(|_| JobsReadError::InvalidQuery("invalid cursor"))?;
            if cursor.version != 1 || cursor.binding != result.binding || cursor.id.len() > 128 {
                return Err(JobsReadError::InvalidQuery("cursor does not match query"));
            }
            result.and(
                field("created_at")
                    .lt(cursor.created_at)
                    .or(field("created_at")
                        .eq(cursor.created_at)
                        .and(field("id").lt(cursor.id))),
            );
        }
        Ok(result)
    }

    fn and(&mut self, expr: SimpleExpr) {
        self.condition = std::mem::replace(&mut self.condition, Condition::all()).add(expr);
    }

    fn equal(&mut self, column: &str, value: Option<&str>) -> Result<(), JobsReadError> {
        if let Some(value) = value {
            validate_text(value)?;
            // `column = ? COLLATE NOCASE`: the bound value carries the collation.
            self.and(field(column).binary(
                BinOper::Equal,
                Expr::cust_with_expr("? COLLATE NOCASE", Expr::val(value)),
            ));
        }
        Ok(())
    }

    fn contains(&mut self, column: &str, value: Option<&str>) -> Result<(), JobsReadError> {
        if let Some(value) = value {
            validate_text(value)?;
            let position = Func::cust(Alias::new("instr"))
                .arg(Func::lower(field(column)))
                .arg(Func::lower(Expr::val(value)));
            self.and(Expr::expr(position).gt(0));
        }
        Ok(())
    }

    pub fn next(&self, created_at: i64, id: &str) -> Result<String, JobsReadError> {
        serde_json::to_vec(&Cursor {
            version: 1,
            binding: self.binding.clone(),
            created_at,
            id: id.into(),
        })
        .map(hex::encode)
        .map_err(|_| JobsReadError::Unavailable)
    }

    /// Trims the `limit + 1` probe row and derives the next cursor from the
    /// last kept item's (created_at, id) when more rows exist.
    pub fn finish<T>(
        &self,
        mut items: Vec<T>,
        key: impl Fn(&T) -> (i64, &str),
    ) -> Result<(Vec<T>, Option<String>), JobsReadError> {
        let more = items.len() > self.limit;
        items.truncate(self.limit);
        let next_cursor = if more {
            items
                .last()
                .map(|item| {
                    let (created_at, id) = key(item);
                    self.next(created_at, id)
                })
                .transpose()?
        } else {
            None
        };
        Ok((items, next_cursor))
    }

    /// Filters `view` and orders it newest first, probing one extra row.
    pub fn select(&self, view: SelectStatement) -> SelectStatement {
        let mut select = super::views::from_view(view);
        select.cond_where(self.condition.clone());
        super::views::keyset(&mut select, Order::Desc)
            .limit(self.limit as u64 + 1)
            .to_owned()
    }
}

fn validate_text(value: &str) -> Result<(), JobsReadError> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        Err(JobsReadError::InvalidQuery("invalid filter text"))
    } else {
        Ok(())
    }
}
