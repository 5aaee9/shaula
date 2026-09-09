//! Core listener ledger port backed by short SQLite transactions.
use async_trait::async_trait;
use shaula_core::error::CoreResult;
use shaula_core::ports::PollMessage;
use shaula_core::registry::{IngestedMessage, ListenerMessageStore, SessionEffectContext};

use super::{core_err, SqliteControlPlane};

#[async_trait]
impl ListenerMessageStore for SqliteControlPlane {
    async fn listener_current(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
    ) -> CoreResult<bool> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let current = self
            .store
            .listener_context_current_tx(&tx, fleet, context)
            .await
            .map_err(core_err)?;
        tx.commit()
            .await
            .map_err(crate::StoreError::from)
            .map_err(core_err)?;
        Ok(current)
    }

    async fn listener_ingest(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message: &PollMessage,
        now: i64,
    ) -> CoreResult<Option<IngestedMessage>> {
        self.store
            .listener_ingest(fleet, context, message, now)
            .await
            .map_err(core_err)
    }

    async fn listener_pending(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
    ) -> CoreResult<Vec<IngestedMessage>> {
        self.store
            .listener_pending(fleet, context)
            .await
            .map_err(core_err)
    }

    async fn listener_acknowledge(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        now: i64,
    ) -> CoreResult<bool> {
        self.store
            .listener_acknowledge(fleet, context, message_id, now)
            .await
            .map_err(core_err)
    }

    async fn listener_acquire_start(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        now: i64,
    ) -> CoreResult<Option<Vec<i64>>> {
        self.store
            .listener_acquire_start(fleet, context, message_id, now)
            .await
            .map_err(core_err)
    }

    async fn listener_acquire_complete(
        &self,
        fleet: &str,
        context: &SessionEffectContext,
        message_id: i64,
        accepted: Option<&[i64]>,
        now: i64,
    ) -> CoreResult<bool> {
        self.store
            .listener_acquire_complete(fleet, context, message_id, accepted, now)
            .await
            .map_err(core_err)
    }
}
