//! Shared audit/outbox/idempotency helpers, split to keep files under
//! 400 lines (AGENTS.md).

use sea_orm::sea_query::Expr;
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use crate::entities::shared::{audit_records, idempotency_records, outbox};
use crate::store::{Store, StoreResult};

impl Store {
    pub(crate) async fn audit_append(
        &self,
        tx: &DatabaseTransaction,
        append: shaula_core::registry::AuditAppend,
    ) -> StoreResult<()> {
        let shaula_core::registry::AuditAppend {
            resource_kind,
            action,
            actor,
            resource_key,
            revision,
            outcome,
            detail_json,
            now,
        } = append;
        let resource_kind = resource_kind.as_str();
        let action = action.as_str();
        let actor = actor.as_str();
        let resource_key = resource_key.as_str();
        let outcome = outcome.as_str();
        let row = audit_records::ActiveModel {
            seq: Default::default(),
            resource_kind: Set(resource_kind.to_string()),
            action: Set(action.to_string()),
            actor: Set(actor.to_string()),
            resource_key: Set(resource_key.to_string()),
            revision: Set(revision),
            outcome: Set(outcome.to_string()),
            detail_json: Set(detail_json),
            created_at: Set(now),
        };
        audit_records::Entity::insert(row).exec(tx).await?;
        Ok(())
    }

    pub(crate) async fn outbox_enqueue(
        &self,
        tx: &DatabaseTransaction,
        resource_kind: &str,
        topic: &str,
        payload: &str,
        now: i64,
    ) -> StoreResult<()> {
        let row = outbox::ActiveModel {
            id: Default::default(),
            resource_kind: Set(resource_kind.to_string()),
            topic: Set(topic.to_string()),
            payload: Set(payload.to_string()),
            created_at: Set(now),
            processed_at: Set(None),
        };
        outbox::Entity::insert(row).exec(tx).await?;
        Ok(())
    }

    /// Marks pending outbox rows processed; returns how many were claimed.
    pub(crate) async fn outbox_flush(&self, now: i64) -> StoreResult<u64> {
        let result = outbox::Entity::update_many()
            .col_expr(outbox::Column::ProcessedAt, Expr::value(now))
            .filter(outbox::Column::ProcessedAt.is_null())
            .exec(self.connection())
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn idempotency_store(
        &self,
        tx: &DatabaseTransaction,
        insert: shaula_core::registry::IdempotencyInsert,
    ) -> StoreResult<()> {
        let row = idempotency_records::ActiveModel {
            id: Set(insert.id),
            resource_kind: Set(insert.resource_kind),
            resource_key: Set(insert.resource_key),
            idempotency_key: Set(insert.idempotency_key),
            request_hash: Set(insert.request_hash),
            response_status: Set(insert.response_status),
            response_body: Set(insert.response_body),
            created_at: Set(insert.now),
        };
        idempotency_records::Entity::insert(row).exec(tx).await?;
        Ok(())
    }
}
