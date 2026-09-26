use super::{row::Row, sql, unavailable, SqliteStateBackend};
use sea_orm::{ConnectionTrait, DatabaseTransaction};
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
        let Some(worker) = super::worker::WorkerRow::read(tx, &proof.generation_id).await? else {
            return Ok(());
        };
        if worker.phase != "running"
            || worker.cleanup_only
            || worker.handover.is_some()
            || worker.active_command.is_some()
        {
            return Err(StateError::Conflict);
        }
        tx.execute(sql(
            "UPDATE lifecycle_workers SET active_command = ? WHERE generation_id = ?",
            vec![
                format!("bootstrap:{}", proof.attempt_id).into(),
                proof.generation_id.clone().into(),
            ],
        ))
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
        let Some(worker) = tx.query_one(sql(
            "SELECT phase, cleanup_only, handover, active_command FROM lifecycle_workers WHERE generation_id = ?",
            vec![proof.generation_id.clone().into()],
        )).await.map_err(unavailable)? else { return Ok(()); };
        if worker.try_get::<String>("", "phase").map_err(unavailable)? != "running"
            || worker
                .try_get::<Option<String>>("", "handover")
                .map_err(unavailable)?
                .is_some()
            || worker
                .try_get::<Option<String>>("", "active_command")
                .map_err(unavailable)?
                .is_some()
        {
            return Err(StateError::Conflict);
        }
        let row = Row::load(tx, &proof.generation_id).await?;
        row.writable()?;
        if row.revoked {
            return Err(StateError::Unauthorized);
        }
        let snapshot = row.snapshot(tx).await?.ok_or(StateError::Unavailable)?;
        if proof.intent == PlanIntent::Create {
            if !snapshot.document.managed_empty() {
                return Err(StateError::Conflict);
            }
            let protected = tx
                .query_one(sql(
                    "SELECT protected_input FROM lifecycle_workers WHERE generation_id = ?",
                    vec![proof.generation_id.clone().into()],
                ))
                .await
                .map_err(unavailable)?
                .ok_or(StateError::Unavailable)?;
            let input = protected
                .try_get::<Option<Vec<u8>>>("", "protected_input")
                .map_err(unavailable)?
                .ok_or(StateError::Conflict)?;
            use sha2::{Digest, Sha256};
            if format!("sha256:{}", hex::encode(Sha256::digest(input)))
                != proof.protected_input_digest
            {
                return Err(StateError::Conflict);
            }
            if row.create_started
                || worker
                    .try_get::<bool>("", "cleanup_only")
                    .map_err(unavailable)?
            {
                return Err(StateError::Conflict);
            }
            tx.execute(sql(
                "UPDATE generation_http_state SET create_started = 1 WHERE generation_id = ?",
                vec![proof.generation_id.clone().into()],
            ))
            .await
            .map_err(unavailable)?;
        }
        tx.execute(sql(
            "UPDATE lifecycle_workers SET handover = ?, active_command = ? WHERE generation_id = ?",
            vec![
                proof.attempt_id.clone().into(),
                proof.attempt_id.clone().into(),
                proof.generation_id.clone().into(),
            ],
        ))
        .await
        .map_err(unavailable)?;
        Ok(())
    }
}
