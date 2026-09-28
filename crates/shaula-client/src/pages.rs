//! Cursor-paginated registry lists (spec 0002 §5.3 / 0005 §4).
use super::*;
use types::Document;

/// Largest page the server accepts; fewer round trips for complete lists.
const PAGE_LIMIT: &str = "200";
/// Upper bound on pages followed by one complete list read.
const MAX_PAGES: usize = 1_000;

impl Client {
    /// One page; `cursor` is the previous page's opaque `next_cursor`.
    pub(crate) async fn read_page(
        &self,
        path: &[&str],
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Resource<Document>, Error> {
        let mut query = Vec::new();
        if let Some(limit) = limit {
            query.push(("limit".to_owned(), limit.to_string()));
        }
        if let Some(cursor) = cursor {
            query.push(("cursor".to_owned(), cursor.to_owned()));
        }
        self.read(path, &query).await
    }

    /// Follows `next_cursor` and merges the `field` arrays into one document
    /// with a null `next_cursor`. A server that sends no cursor is one page.
    pub(crate) async fn read_all_pages(
        &self,
        path: &[&str],
        field: &str,
    ) -> Result<Resource<Document>, Error> {
        let mut items = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut query = vec![("limit".to_owned(), PAGE_LIMIT.to_owned())];
        for _ in 0..MAX_PAGES {
            let page: Resource<serde_json::Value> = self.read(path, &query).await?;
            let mut page = page.data;
            let entries = page
                .get_mut(field)
                .and_then(serde_json::Value::as_array_mut)
                .ok_or(Error::Protocol)?;
            items.append(entries);
            let Some(next) = page.get("next_cursor").and_then(|v| v.as_str()) else {
                page[field] = serde_json::Value::Array(items);
                page["next_cursor"] = serde_json::Value::Null;
                let data = Document::from_serializable(page).map_err(|_| Error::Protocol)?;
                return Ok(Resource {
                    data,
                    version: None,
                });
            };
            if !seen.insert(next.to_owned()) {
                return Err(Error::Protocol);
            }
            query.retain(|(key, _)| key != "cursor");
            query.push(("cursor".to_owned(), next.to_owned()));
        }
        Err(Error::Protocol)
    }
}
