//! Atomic message facts and acknowledged checkpoints; no network calls.
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, QueryResult, Statement};
use sha2::Digest;
use shaula_core::ports::PollMessage;
use shaula_core::registry::{IngestedMessage, SessionEffectContext};

use crate::{Store, StoreError, StoreResult};

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
        let payload = serde_json::to_vec(message)
            .map_err(|_| StoreError::Corrupt("listener message cannot be encoded".into()))?;
        let digest = hex::encode(sha2::Sha256::digest(payload));
        if let Some(row) = self
            .listener_message_tx(&tx, fleet, context, message.message_id)
            .await?
        {
            if row.try_get::<String>("", "payload_digest")? != digest {
                return Err(StoreError::Corrupt(
                    "redelivered listener message changed contents".into(),
                ));
            }
            let result = self
                .listener_message_view_tx(&tx, fleet, context.epoch, &row)
                .await?;
            tx.commit().await?;
            return Ok(Some(result));
        }
        let latest = tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT MAX(message_id) AS latest FROM listener_messages WHERE fleet_key=? AND epoch=?",
            [fleet.into(), context.epoch.into()])).await?;
        if latest
            .map(|r| r.try_get::<Option<i64>>("", "latest"))
            .transpose()?
            .flatten()
            .is_some_and(|id| id >= message.message_id)
        {
            return Err(StoreError::Corrupt(
                "listener message order regressed".into(),
            ));
        }
        let json = serde_json::to_string(&context.auth_context)
            .map_err(|_| StoreError::Corrupt("listener context cannot be encoded".into()))?;
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO listener_messages(fleet_key,epoch,message_id,payload_digest,incarnation,
             fleet_revision,mutation_fence,profile_key,auth_revision,context_json,created_at,updated_at)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            [fleet.into(),context.epoch.into(),message.message_id.into(),digest.into(),
             context.guard.incarnation.clone().into(),context.guard.desired_revision.into(),
             context.guard.mutation_fence.into(),context.auth_context.profile_key.clone().into(),
             context.auth_context.revision.into(),json.into(),now.into(),now.into()])).await?;
        tx.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO fleet_demand(fleet_key,total_assigned_jobs,updated_at) VALUES(?,?,?)
             ON CONFLICT(fleet_key) DO UPDATE SET total_assigned_jobs=excluded.total_assigned_jobs,
             updated_at=excluded.updated_at",
            [
                fleet.into(),
                message.statistics.total_assigned_jobs.into(),
                now.into(),
            ],
        ))
        .await?;
        self.listener_observations_tx(&tx, fleet, context.epoch, message, now)
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

    pub(crate) async fn listener_message_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
    ) -> StoreResult<Option<QueryResult>> {
        let row = tx
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT * FROM listener_messages WHERE fleet_key=? AND epoch=? AND message_id=?",
                [fleet.into(), context.epoch.into(), message_id.into()],
            ))
            .await?;
        if let Some(row) = &row {
            let json: String = row.try_get("", "context_json")?;
            let saved: shaula_core::auth_context::ResolvedAuthContext = serde_json::from_str(&json)
                .map_err(|_| StoreError::Corrupt("listener context corrupt".into()))?;
            if row.try_get::<String>("", "incarnation")? != context.guard.incarnation
                || row.try_get::<i64>("", "fleet_revision")? != context.guard.desired_revision
                || row.try_get::<i64>("", "mutation_fence")? != context.guard.mutation_fence
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
        row: &QueryResult,
    ) -> StoreResult<IngestedMessage> {
        let message_id: i64 = row.try_get("", "message_id")?;
        let pending = tx
            .query_all(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT runner_request_id FROM listener_acquisitions WHERE fleet_key=? AND epoch=?
             AND message_id=? AND state='Pending' ORDER BY runner_request_id",
                [fleet.into(), epoch.into(), message_id.into()],
            ))
            .await?;
        Ok(IngestedMessage {
            message_id,
            acked: row.try_get::<i64>("", "acked")? != 0,
            pending_request_ids: pending
                .into_iter()
                .map(|r| r.try_get("", "runner_request_id").map_err(StoreError::from))
                .collect::<StoreResult<_>>()?,
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
        let rows = tx.query_all(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT m.* FROM listener_messages m WHERE m.fleet_key=? AND m.epoch=?
             AND (m.acked=0 OR EXISTS(SELECT 1 FROM listener_acquisitions a WHERE
             a.fleet_key=m.fleet_key AND a.epoch=m.epoch AND a.message_id=m.message_id AND a.state='Pending'))
             ORDER BY m.message_id",
            [fleet.into(),context.epoch.into()])).await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            self.listener_message_tx(&tx, fleet, context, row.try_get("", "message_id")?)
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
        let earlier = tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT message_id FROM listener_messages WHERE fleet_key=? AND epoch=? AND message_id<? AND acked=0 LIMIT 1",
            [fleet.into(),context.epoch.into(),message_id.into()])).await?;
        if earlier.is_some() {
            return Err(StoreError::Corrupt(
                "listener ACK would skip an unacknowledged message".into(),
            ));
        }
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "UPDATE listener_messages SET acked=1,updated_at=? WHERE fleet_key=? AND epoch=? AND message_id=?",
            [now.into(),fleet.into(),context.epoch.into(),message_id.into()])).await?;
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "UPDATE fleet_sessions SET last_message_id=MAX(last_message_id,?) WHERE fleet_key=? AND epoch=?",
            [message_id.into(),fleet.into(),context.epoch.into()])).await?;
        tx.commit().await?;
        Ok(true)
    }
}
