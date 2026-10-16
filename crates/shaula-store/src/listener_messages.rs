//! Atomic message facts and acknowledged checkpoints; no network calls.
use sea_orm::{
    sea_query::{Alias, Expr, Func, OnConflict},
    ActiveValue::Set,
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use shaula_core::ports::PollMessage;
use shaula_core::registry::{IngestedMessage, SessionEffectContext};

use crate::entities::lifecycle::{fleet_demand, fleet_sessions};
use crate::entities::listener::{
    listener_acquisitions as acquisitions, listener_messages as messages,
};
use crate::query_ext::{col, probe, same};
use crate::{Store, StoreError, StoreResult};

#[path = "listener_digest.rs"]
mod digest;

impl Store {
    pub(crate) async fn listener_context_current_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        context: &SessionEffectContext,
    ) -> StoreResult<bool> {
        self.session_guard_tx(
            tx,
            fleet,
            &context.guard,
            &context.auth_context,
            context.epoch,
        )
        .await
    }

    pub(crate) async fn listener_ingest(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message: &PollMessage,
        now: i64,
    ) -> StoreResult<Option<IngestedMessage>> {
        let tx = self.begin().await?;
        if !self
            .listener_context_current_tx(&tx, fleet, context)
            .await?
        {
            return Ok(None);
        }
        if message.message_id <= 0 || message.statistics.total_assigned_jobs < 0 {
            return Err(StoreError::Corrupt(
                "invalid listener message statistics or ID".into(),
            ));
        }
        let digest = digest::current(message)?;
        if let Some(row) = self
            .listener_message_tx(&tx, fleet, context, message.message_id)
            .await?
        {
            let version = row.payload_digest_version;
            let expected = match version {
                1 => digest::legacy(message)?,
                2 => digest.clone(),
                _ => {
                    return Err(StoreError::Corrupt(
                        "unsupported listener digest version".into(),
                    ))
                }
            };
            if row.payload_digest != expected {
                return Err(StoreError::Corrupt(
                    "redelivered listener message changed contents".into(),
                ));
            }
            if version == 1 {
                // Only a real, currently authorized re-delivery can supply the
                // metadata omitted by the old adapter. Freeze it exactly once.
                self.jobs_ingest_tx(&tx, fleet, context, message, now)
                    .await?;
                messages::Entity::update_many()
                    .col_expr(messages::Column::PayloadDigest, Expr::value(digest.clone()))
                    .col_expr(messages::Column::PayloadDigestVersion, Expr::value(2))
                    .filter(messages::Column::FleetKey.eq(fleet))
                    .filter(messages::Column::Epoch.eq(context.epoch))
                    .filter(messages::Column::MessageId.eq(message.message_id))
                    .filter(messages::Column::PayloadDigestVersion.eq(1))
                    .exec(&tx)
                    .await?;
            }
            let result = self
                .listener_message_view_tx(&tx, fleet, context.epoch, &row)
                .await?;
            tx.commit().await?;
            return Ok(Some(result));
        }
        let latest = messages::Entity::find()
            .select_only()
            .expr(col(messages::Column::MessageId).max())
            .filter(messages::Column::FleetKey.eq(fleet))
            .filter(messages::Column::Epoch.eq(context.epoch))
            .into_tuple::<Option<i64>>()
            .one(&tx)
            .await?
            .flatten();
        if latest.is_some_and(|id| id >= message.message_id) {
            return Err(StoreError::Corrupt(
                "listener message order regressed".into(),
            ));
        }
        let json = serde_json::to_string(&context.auth_context)
            .map_err(|_| StoreError::Corrupt("listener context cannot be encoded".into()))?;
        messages::Entity::insert(messages::ActiveModel {
            fleet_key: Set(fleet.to_owned()),
            epoch: Set(context.epoch),
            message_id: Set(message.message_id),
            payload_digest: Set(digest),
            payload_digest_version: Set(2),
            incarnation: Set(context.guard.incarnation.clone()),
            fleet_revision: Set(context.guard.desired_revision),
            mutation_fence: Set(context.guard.mutation_fence),
            profile_key: Set(context.auth_context.profile_key.clone()),
            auth_revision: Set(context.auth_context.revision),
            context_json: Set(json),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec_without_returning(&tx)
        .await?;
        fleet_demand::Entity::insert(fleet_demand::ActiveModel {
            fleet_key: Set(fleet.to_owned()),
            total_assigned_jobs: Set(message.statistics.total_assigned_jobs),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(fleet_demand::Column::FleetKey)
                .update_columns([
                    fleet_demand::Column::TotalAssignedJobs,
                    fleet_demand::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(&tx)
        .await?;
        self.listener_observations_tx(&tx, fleet, context.epoch, message, now)
            .await?;
        self.listener_activity_tx(&tx, fleet, message, now).await?;
        self.jobs_ingest_tx(&tx, fleet, context, message, now)
            .await?;
        self.outbox_enqueue(
            &tx,
            "fleet",
            "fleet.listener",
            &serde_json::json!({"fleet_key":fleet,"epoch":context.epoch}).to_string(),
            now,
        )
        .await?;
        let row = self
            .listener_message_tx(&tx, fleet, context, message.message_id)
            .await?
            .ok_or_else(|| StoreError::Corrupt("listener message insert missing".into()))?;
        let result = self
            .listener_message_view_tx(&tx, fleet, context.epoch, &row)
            .await?;
        tx.commit().await?;
        Ok(Some(result))
    }

    pub(crate) async fn listener_demand_confirmed(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.begin().await?;
        if !self
            .listener_context_current_tx(&tx, fleet, context)
            .await?
        {
            return Ok(false);
        }
        let touched = fleet_demand::Entity::update_many()
            .col_expr(fleet_demand::Column::UpdatedAt, Expr::value(now))
            .filter(fleet_demand::Column::FleetKey.eq(fleet))
            .filter(fleet_demand::Column::UpdatedAt.lt(now))
            .exec(&tx)
            .await?
            .rows_affected;
        tx.commit().await?;
        Ok(touched > 0)
    }

    pub(crate) async fn listener_message_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
    ) -> StoreResult<Option<messages::Model>> {
        let row = messages::Entity::find_by_id((fleet.to_owned(), context.epoch, message_id))
            .one(tx)
            .await?;
        if let Some(row) = &row {
            let saved: shaula_core::auth_context::ResolvedAuthContext =
                serde_json::from_str(&row.context_json)
                    .map_err(|_| StoreError::Corrupt("listener context corrupt".into()))?;
            if row.incarnation != context.guard.incarnation
                || row.fleet_revision != context.guard.desired_revision
                || row.mutation_fence != context.guard.mutation_fence
                || saved != context.auth_context
            {
                return Err(StoreError::Corrupt(
                    "listener message authority mismatch".into(),
                ));
            }
        }
        Ok(row)
    }

    pub(crate) async fn listener_message_view_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        epoch: i64,
        row: &messages::Model,
    ) -> StoreResult<IngestedMessage> {
        let pending_request_ids = acquisitions::Entity::find()
            .select_only()
            .column(acquisitions::Column::RunnerRequestId)
            .filter(acquisitions::Column::FleetKey.eq(fleet))
            .filter(acquisitions::Column::Epoch.eq(epoch))
            .filter(acquisitions::Column::MessageId.eq(row.message_id))
            .filter(acquisitions::Column::State.eq("Pending"))
            .order_by_asc(acquisitions::Column::RunnerRequestId)
            .into_tuple::<i64>()
            .all(tx)
            .await?;
        Ok(IngestedMessage {
            message_id: row.message_id,
            acked: row.acked != 0,
            pending_request_ids,
        })
    }

    pub(crate) async fn listener_pending(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
    ) -> StoreResult<Vec<IngestedMessage>> {
        let tx = self.begin().await?;
        if !self
            .listener_context_current_tx(&tx, fleet, context)
            .await?
        {
            return Ok(vec![]);
        }
        let pending = probe(acquisitions::Entity)
            .and_where(same(
                acquisitions::Column::FleetKey,
                messages::Column::FleetKey,
            ))
            .and_where(same(acquisitions::Column::Epoch, messages::Column::Epoch))
            .and_where(same(
                acquisitions::Column::MessageId,
                messages::Column::MessageId,
            ))
            .and_where(col(acquisitions::Column::State).eq("Pending"))
            .to_owned();
        let rows = messages::Entity::find()
            .filter(messages::Column::FleetKey.eq(fleet))
            .filter(messages::Column::Epoch.eq(context.epoch))
            .filter(col(messages::Column::Acked).eq(0).or(Expr::exists(pending)))
            .order_by_asc(messages::Column::MessageId)
            .all(&tx)
            .await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            self.listener_message_tx(&tx, fleet, context, row.message_id)
                .await?;
            result.push(
                self.listener_message_view_tx(&tx, fleet, context.epoch, &row)
                    .await?,
            );
        }
        tx.commit().await?;
        Ok(result)
    }

    pub(crate) async fn listener_acknowledge(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.begin().await?;
        if !self
            .listener_context_current_tx(&tx, fleet, context)
            .await?
            || self
                .listener_message_tx(&tx, fleet, context, message_id)
                .await?
                .is_none()
        {
            return Ok(false);
        }
        let earlier = messages::Entity::find()
            .select_only()
            .column(messages::Column::MessageId)
            .filter(messages::Column::FleetKey.eq(fleet))
            .filter(messages::Column::Epoch.eq(context.epoch))
            .filter(messages::Column::MessageId.lt(message_id))
            .filter(messages::Column::Acked.eq(0))
            .limit(1)
            .into_tuple::<i64>()
            .one(&tx)
            .await?;
        if earlier.is_some() {
            return Err(StoreError::Corrupt(
                "listener ACK would skip an unacknowledged message".into(),
            ));
        }
        messages::Entity::update_many()
            .col_expr(messages::Column::Acked, Expr::value(1))
            .col_expr(messages::Column::UpdatedAt, Expr::value(now))
            .filter(messages::Column::FleetKey.eq(fleet))
            .filter(messages::Column::Epoch.eq(context.epoch))
            .filter(messages::Column::MessageId.eq(message_id))
            .exec(&tx)
            .await?;
        // SQLite's two-argument MAX is a scalar, not the aggregate.
        let advanced = Func::cust(Alias::new("MAX"))
            .arg(col(fleet_sessions::Column::LastMessageId))
            .arg(Expr::value(message_id));
        fleet_sessions::Entity::update_many()
            .col_expr(fleet_sessions::Column::LastMessageId, advanced.into())
            .filter(fleet_sessions::Column::FleetKey.eq(fleet))
            .filter(fleet_sessions::Column::Epoch.eq(context.epoch))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}
