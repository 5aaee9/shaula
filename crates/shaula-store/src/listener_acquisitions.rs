//! Durable at-most-once acquisition attempts and old-session classification.
use sea_orm::{
    sea_query::{Expr, SimpleExpr},
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::registry::SessionEffectContext;

use crate::entities::listener::listener_acquisitions as acquisitions;
use crate::query_ext::col;
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
        if row.acked == 0 {
            return Err(StoreError::Corrupt("acquisition before message ACK".into()));
        }
        let ids = self
            .listener_message_view_tx(&tx, fleet, context.epoch, &row)
            .await?
            .pending_request_ids;
        acquisitions::Entity::update_many()
            .col_expr(acquisitions::Column::State, Expr::value("AcquireStarting"))
            .col_expr(acquisitions::Column::UpdatedAt, Expr::value(now))
            .filter(acquisitions::Column::FleetKey.eq(fleet))
            .filter(acquisitions::Column::Epoch.eq(context.epoch))
            .filter(acquisitions::Column::MessageId.eq(message_id))
            .filter(acquisitions::Column::State.eq("Pending"))
            .exec(&tx)
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
        let rows = acquisitions::Entity::find()
            .select_only()
            .columns([
                acquisitions::Column::RunnerRequestId,
                acquisitions::Column::State,
            ])
            .filter(acquisitions::Column::FleetKey.eq(fleet))
            .filter(acquisitions::Column::Epoch.eq(context.epoch))
            .filter(acquisitions::Column::MessageId.eq(message_id))
            .into_tuple::<(i64, String)>()
            .all(&tx)
            .await?;
        let ids = rows.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        if accepted.is_some_and(|values| values.iter().any(|id| !ids.contains(id))) {
            return Err(StoreError::Corrupt(
                "acquisition result includes an unrequested job".into(),
            ));
        }
        for (id, previous) in rows {
            if !matches!(previous.as_str(), "AcquireStarting" | "Uncertain") {
                continue;
            }
            let state = match accepted {
                None => "Uncertain",
                Some(ids) if ids.contains(&id) => "Acquired",
                Some(_) => "Rejected",
            };
            acquisitions::Entity::update_many()
                .col_expr(acquisitions::Column::State, Expr::value(state))
                .col_expr(acquisitions::Column::UpdatedAt, Expr::value(now))
                .filter(acquisitions::Column::FleetKey.eq(fleet))
                .filter(acquisitions::Column::Epoch.eq(context.epoch))
                .filter(acquisitions::Column::RunnerRequestId.eq(id))
                .exec(&tx)
                .await?;
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
        let ended: SimpleExpr =
            Expr::case(col(acquisitions::Column::State).eq("Pending"), "Cancelled")
                .finally("Uncertain")
                .into();
        acquisitions::Entity::update_many()
            .col_expr(acquisitions::Column::State, ended)
            .filter(acquisitions::Column::FleetKey.eq(fleet))
            .filter(acquisitions::Column::Epoch.eq(epoch))
            .filter(acquisitions::Column::State.is_in(["Pending", "AcquireStarting"]))
            .exec(tx)
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
        acquisitions::Entity::update_many()
            .col_expr(
                acquisitions::Column::State,
                Expr::value("ReconciledUncertain"),
            )
            .col_expr(acquisitions::Column::UpdatedAt, Expr::value(now))
            .filter(acquisitions::Column::FleetKey.eq(fleet))
            .filter(acquisitions::Column::Epoch.lt(epoch))
            .filter(acquisitions::Column::State.eq("Uncertain"))
            .exec(tx)
            .await?;
        Ok(())
    }
}
