//! Durable at-most-once acquisition attempts and old-session classification.
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};
use shaula_core::registry::SessionEffectContext;

use crate::{Store, StoreError, StoreResult};

impl Store {
    pub(crate) async fn listener_acquire_start(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        now: i64,
    ) -> StoreResult<Option<Vec<i64>>> {
        let tx = self.begin().await?;
        if !self
            .listener_context_current_tx(&tx, fleet, context)
            .await?
        {
            return Ok(None);
        }
        let Some(row) = self
            .listener_message_tx(&tx, fleet, context, message_id)
            .await?
        else {
            return Err(StoreError::Corrupt(
                "acquisition message not persisted".into(),
            ));
        };
        if row.try_get::<i64>("", "acked")? == 0 {
            return Err(StoreError::Corrupt("acquisition before message ACK".into()));
        }
        let ids = self
            .listener_message_view_tx(&tx, fleet, context.epoch, &row)
            .await?
            .pending_request_ids;
        tx.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE listener_acquisitions SET state='AcquireStarting',updated_at=?
             WHERE fleet_key=? AND epoch=? AND message_id=? AND state='Pending'",
            [
                now.into(),
                fleet.into(),
                context.epoch.into(),
                message_id.into(),
            ],
        ))
        .await?;
        tx.commit().await?;
        Ok(Some(ids))
    }

    pub(crate) async fn listener_acquire_complete(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        accepted: Option<&[i64]>,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.begin().await?;
        if self
            .listener_message_tx(&tx, fleet, context, message_id)
            .await?
            .is_none()
        {
            return Err(StoreError::Corrupt(
                "acquisition completion message missing".into(),
            ));
        }
        let rows = tx.query_all(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT runner_request_id,state FROM listener_acquisitions WHERE fleet_key=? AND epoch=? AND message_id=?",
            [fleet.into(),context.epoch.into(),message_id.into()])).await?;
        let ids = rows
            .iter()
            .map(|r| {
                r.try_get::<i64>("", "runner_request_id")
                    .map_err(StoreError::from)
            })
            .collect::<StoreResult<Vec<_>>>()?;
        if accepted.is_some_and(|values| values.iter().any(|id| !ids.contains(id))) {
            return Err(StoreError::Corrupt(
                "acquisition result includes an unrequested job".into(),
            ));
        }
        for row in rows {
            let id: i64 = row.try_get("", "runner_request_id")?;
            let previous: String = row.try_get("", "state")?;
            if !matches!(previous.as_str(), "AcquireStarting" | "Uncertain") {
                continue;
            }
            let state = match accepted {
                None => "Uncertain",
                Some(ids) if ids.contains(&id) => "Acquired",
                Some(_) => "Rejected",
            };
            tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
                "UPDATE listener_acquisitions SET state=?,updated_at=? WHERE fleet_key=? AND epoch=? AND runner_request_id=?",
                [state.into(),now.into(),fleet.into(),context.epoch.into(),id.into()])).await?;
        }
        // A corrupt or replaced current authority cannot erase the outcome
        // of an effect that already reached GitHub. Commit it before returning
        // any current-authority validation error.
        let current = self.listener_context_current_tx(&tx, fleet, context).await;
        tx.commit().await?;
        current
    }

    /// Called under the exclusive session-effect gate. Pending never left
    /// the daemon; Starting may have reached GitHub and must remain uncertain.
    pub(crate) async fn listener_session_end_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        epoch: i64,
    ) -> StoreResult<()> {
        tx.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE listener_acquisitions SET state=CASE state
             WHEN 'Pending' THEN 'Cancelled' ELSE 'Uncertain' END
             WHERE fleet_key=? AND epoch=? AND state IN ('Pending','AcquireStarting')",
            [fleet.into(), epoch.into()],
        ))
        .await?;
        Ok(())
    }

    /// A fresh replacement session's authoritative initial statistics now
    /// cover demand. Keep the historical outcome; release only its live pin.
    pub(crate) async fn listener_reconcile_old_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet: &str,
        epoch: i64,
        now: i64,
    ) -> StoreResult<()> {
        tx.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE listener_acquisitions SET state='ReconciledUncertain',updated_at=?
             WHERE fleet_key=? AND epoch<? AND state='Uncertain'",
            [now.into(), fleet.into(), epoch.into()],
        ))
        .await?;
        Ok(())
    }
}
