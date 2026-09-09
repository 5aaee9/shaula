use super::{unavailable, OperationLogArchive};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::{
    error::CoreResult,
    operation_log::{Invocation, InvocationQuery, InvocationsPage},
};

impl OperationLogArchive {
    pub(super) async fn invocations_page(
        &self,
        generation_id: &str,
        query: InvocationQuery,
    ) -> CoreResult<InvocationsPage> {
        let limit = query.page_limit()?;
        let position = query.position_for(generation_id)?;
        let mut values = vec![generation_id.into()];
        let mut sql = "SELECT id FROM operation_log_invocations WHERE generation_id=?".to_string();
        if let Some((started_at, id)) = position {
            sql.push_str(" AND (json_extract(record_json,'$.started_at'),id)<(?,?)");
            values.extend([started_at.into(), id.into()]);
        }
        sql.push_str(" ORDER BY json_extract(record_json,'$.started_at') DESC,id DESC LIMIT ?");
        values.push(i64::try_from(limit + 1).map_err(|_| unavailable())?.into());
        let _reader = self.writer.lock().await;
        let rows = self
            .store
            .connection()
            .query_all(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                sql,
                values,
            ))
            .await
            .map_err(|_| unavailable())?;
        let more = rows.len() > limit;
        let mut items = Vec::with_capacity(limit);
        for row in rows.into_iter().take(limit) {
            items.push(
                self.load(&row.try_get::<String>("", "id").map_err(|_| unavailable())?)
                    .await?,
            );
        }
        let next_cursor = if more {
            items
                .last()
                .map(|item| InvocationQuery::cursor_after(generation_id, item))
                .transpose()?
        } else {
            None
        };
        Ok(InvocationsPage {
            items,
            next_cursor,
            latest_create: self.latest_operation(generation_id, "Create").await?,
            latest_destroy: self.latest_operation(generation_id, "Destroy").await?,
        })
    }

    async fn latest_operation(
        &self,
        generation_id: &str,
        operation: &str,
    ) -> CoreResult<Option<Invocation>> {
        let row = self.store.connection().query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT id FROM operation_log_invocations WHERE generation_id=? AND operation=? ORDER BY json_extract(record_json,'$.started_at') DESC,ordinal DESC,id DESC LIMIT 1", [generation_id.into(),operation.into()])).await.map_err(|_| unavailable())?;
        match row {
            Some(row) => Ok(Some(
                self.load(&row.try_get::<String>("", "id").map_err(|_| unavailable())?)
                    .await?,
            )),
            None => Ok(None),
        }
    }
}
