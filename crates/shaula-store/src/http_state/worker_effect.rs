use super::{row::Row, states, unavailable, worker::WorkerRow, workers, SqliteStateBackend};
use sea_orm::{sea_query::Expr, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use shaula_core::{
    plan::PlanIntent,
    ports::PlanProvenance,
    state_backend::{StateError, StateResult},
};

impl SqliteStateBackend {
    pub(crate) async fn worker_bootstrap_on(
        tx: &DatabaseTransaction,
        proof: &PlanProvenance,
    ) -> StateResult<()> {
        let Some(worker) = WorkerRow::read(tx, &proof.generation_id).await? else {
            return Ok(());
        };
        if worker.phase != "running"
            || worker.cleanup_only
            || worker.handover.is_some()
            || worker.active_command.is_some()
        {
            return Err(StateError::Conflict);
        }
        workers::Entity::update_many()
            .col_expr(
                workers::Column::ActiveCommand,
                Expr::value(format!("bootstrap:{}", proof.attempt_id)),
            )
            .filter(workers::Column::GenerationId.eq(&proof.generation_id))
            .exec(tx)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
    /// Called inside the existing Fleet-fenced ApplyStarting transaction.
    /// No worker row means legacy/static test execution, not a new admission.
    pub(crate) async fn worker_effect_on(
        tx: &DatabaseTransaction,
        proof: &PlanProvenance,
    ) -> StateResult<()> {
        let Some(worker) = WorkerRow::read(tx, &proof.generation_id).await? else {
            return Ok(());
        };
        if worker.phase != "running" || worker.handover.is_some() || worker.active_command.is_some()
        {
            return Err(StateError::Conflict);
        }
        let row = Row::load(tx, &proof.generation_id).await?;
        row.writable()?;
        if row.revoked {
            return Err(StateError::Unauthorized);
        }
        let snapshot = row.snapshot(tx).await?.ok_or(StateError::Unavailable)?;
        if proof.intent == PlanIntent::Destroy
            && row.create_started
            && !super::create_proof::verified(tx, &proof.generation_id, &snapshot.document, false)
                .await?
        {
            return Err(StateError::Conflict);
        }
        if proof.intent == PlanIntent::Create {
            if !snapshot.document.managed_empty() {
                return Err(StateError::Conflict);
            }
            let input = WorkerRow::protected_input(tx, &proof.generation_id)
                .await?
                .ok_or(StateError::Conflict)?;
            use sha2::{Digest, Sha256};
            if format!("sha256:{}", hex::encode(Sha256::digest(input)))
                != proof.protected_input_digest
            {
                return Err(StateError::Conflict);
            }
            if row.create_started || worker.cleanup_only {
                return Err(StateError::Conflict);
            }
            states::Entity::update_many()
                .col_expr(states::Column::CreateStarted, Expr::value(true))
                .filter(states::Column::GenerationId.eq(&proof.generation_id))
                .exec(tx)
                .await
                .map_err(unavailable)?;
        }
        workers::Entity::update_many()
            .col_expr(workers::Column::Handover, Expr::value(&proof.attempt_id))
            .col_expr(
                workers::Column::ActiveCommand,
                Expr::value(&proof.attempt_id),
            )
            .col_expr(
                workers::Column::CleanupRevision,
                Expr::value(Option::<i64>::None),
            )
            .filter(workers::Column::GenerationId.eq(&proof.generation_id))
            .exec(tx)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
}
