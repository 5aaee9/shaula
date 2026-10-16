use super::{unavailable, OperationLogArchive};
use crate::entities::operation_logs::operation_log_invocations as log_invocations;
use crate::query_ext::json_extract;
use sea_orm::{
    sea_query::{Expr, SimpleExpr},
    ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect,
};
use shaula_core::{
    error::CoreResult,
    operation_log::{Invocation, InvocationQuery, InvocationsPage},
};

/// The invocation's own `started_at`, kept inside its sanitized record.
pub(super) fn started_at() -> SimpleExpr {
    json_extract(log_invocations::Column::RecordJson, "$.started_at")
}

impl OperationLogArchive {
    pub(super) async fn invocations_page(
        &self,
        generation_id: &str,
        query: InvocationQuery,
    ) -> CoreResult<InvocationsPage> {
        let limit = query.page_limit()?;
        let position = query.position_for(generation_id)?;
        let mut select = log_invocations::Entity::find()
            .select_only()
            .column(log_invocations::Column::Id)
            .filter(log_invocations::Column::GenerationId.eq(generation_id));
        if let Some((started, id)) = position {
            select = select.filter(
                Expr::tuple([started_at(), Expr::col(log_invocations::Column::Id).into()]).lt(
                    Expr::tuple([Expr::val(started).into(), Expr::val(id).into()]),
                ),
            );
        }
        let select = select
            .order_by(started_at(), Order::Desc)
            .order_by_desc(log_invocations::Column::Id)
            .limit(u64::try_from(limit + 1).map_err(|_| unavailable())?);
        let _reader = self.writer.lock().await;
        let ids = select
            .into_tuple::<String>()
            .all(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        let more = ids.len() > limit;
        let mut items = Vec::with_capacity(limit);
        for id in ids.into_iter().take(limit) {
            items.push(self.load(&id).await?);
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
        let id = log_invocations::Entity::find()
            .select_only()
            .column(log_invocations::Column::Id)
            .filter(log_invocations::Column::GenerationId.eq(generation_id))
            .filter(log_invocations::Column::Operation.eq(operation))
            .order_by(started_at(), Order::Desc)
            .order_by_desc(log_invocations::Column::Ordinal)
            .order_by_desc(log_invocations::Column::Id)
            .into_tuple::<String>()
            .one(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        match id {
            Some(id) => Ok(Some(self.load(&id).await?)),
            None => Ok(None),
        }
    }
}
