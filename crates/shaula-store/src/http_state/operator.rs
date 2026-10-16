use super::{
    fences, row::Row, states, unavailable, worker::WorkerRow, workers, SqliteStateBackend,
};
use sea_orm::{
    sea_query::Expr, ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
};
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
            seal(tx, id).await?;
            return Ok(());
        };
        let state = Row::load(tx, id).await?;
        let quarantined_fenced = worker.phase == "quarantined"
            && fences::Entity::find()
                .filter(fences::Column::GenerationId.eq(id))
                .filter(fences::Column::WorkerAttempt.eq(&state.worker_attempt))
                .filter(fences::Column::WorkerEpoch.eq(state.worker_epoch))
                .filter(
                    fences::Column::Outcome
                        .eq("fenced")
                        .or(fences::Column::ConfirmedFenced.eq(true)),
                )
                .count(tx)
                .await
                .map_err(unavailable)?
                > 0;
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
        seal(tx, id).await?;
        let receipt = serde_json::to_string(&receipt).map_err(|_| StateError::Invalid)?;
        workers::Entity::update_many()
            .col_expr(workers::Column::Phase, Expr::value("completed"))
            .col_expr(workers::Column::CompletionRequest, Expr::value(request))
            .col_expr(workers::Column::CompletionReceipt, Expr::value(receipt))
            .filter(workers::Column::GenerationId.eq(id))
            .exec(tx)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
}

async fn seal(tx: &DatabaseTransaction, id: &str) -> StateResult<()> {
    states::Entity::update_many()
        .col_expr(states::Column::Sealed, Expr::value(true))
        .col_expr(states::Column::Revoked, Expr::value(true))
        .filter(states::Column::GenerationId.eq(id))
        .exec(tx)
        .await
        .map_err(unavailable)?;
    Ok(())
}
