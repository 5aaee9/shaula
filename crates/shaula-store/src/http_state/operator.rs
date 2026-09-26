use super::{row::Row, sql, unavailable, worker::WorkerRow, SqliteStateBackend};
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use shaula_core::{
    state_backend::{StateClaim, StateError, StateResult},
    worker::{CompletionKind, CompletionReceipt},
};
use uuid::Uuid;

impl SqliteStateBackend {
    /// Invoked only inside the existing audited operator Finalize transaction.
    /// An operator's resource attestation does not authorize a live executor.
    pub(crate) async fn operator_seal_on(
        tx: &DatabaseTransaction,
        id: &str,
        request: &str,
        now: i64,
    ) -> StateResult<()> {
        let Some(worker) = WorkerRow::read(tx, id).await? else {
            tx.execute(sql(
                "UPDATE generation_http_state SET sealed = 1, revoked = 1 WHERE generation_id = ?",
                vec![id.into()],
            ))
            .await
            .map_err(unavailable)?;
            return Ok(());
        };
        let state = Row::load(tx, id).await?;
        let quarantined_fenced = worker.phase == "quarantined" && tx.query_one(sql(
            "SELECT 1 FROM lifecycle_fences WHERE generation_id = ? AND worker_attempt = ? AND worker_epoch = ? AND (outcome = 'fenced' OR confirmed_fenced = 1)",
            vec![id.into(), state.worker_attempt.clone().into(), state.worker_epoch.into()],
        )).await.map_err(unavailable)?.is_some();
        if (!matches!(worker.phase.as_str(), "admitted" | "fenced") && !quarantined_fenced)
            || worker.handover.is_some()
            || worker.active_command.is_some()
        {
            return Err(StateError::Conflict);
        }
        let receipt = CompletionReceipt {
            claim: StateClaim {
                generation_id: Uuid::parse_str(id).map_err(|_| StateError::Invalid)?,
                worker_epoch: state.worker_epoch,
                worker_attempt: Uuid::parse_str(&state.worker_attempt)
                    .map_err(|_| StateError::Invalid)?,
            },
            request_id: Uuid::parse_str(request).map_err(|_| StateError::Invalid)?,
            state_revision: state.revision,
            kind: CompletionKind::OperatorAttested,
            completed_at: now,
        };
        tx.execute(sql(
            "UPDATE generation_http_state SET sealed = 1, revoked = 1 WHERE generation_id = ?",
            vec![id.into()],
        ))
        .await
        .map_err(unavailable)?;
        tx.execute(sql("UPDATE lifecycle_workers SET phase = 'completed', completion_request = ?, completion_receipt = ? WHERE generation_id = ?", vec![request.into(), serde_json::to_string(&receipt).map_err(|_| StateError::Invalid)?.into(), id.into()])).await.map_err(unavailable)?;
        Ok(())
    }
}
