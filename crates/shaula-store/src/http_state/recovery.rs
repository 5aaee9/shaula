//! Restart classification under the daemon ownership lock, before acquisition.
use super::{
    fences, null, row::Row, set_worker, states, unavailable, worker::WorkerRow, workers,
    SqliteStateBackend, WorkerAdmissions,
};
use crate::entities::lifecycle::runner_generations as generations;
use crate::query_ext::{col, same, JoinOn};
use sea_orm::{
    sea_query::Expr, ActiveValue::Set, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use shaula_core::{
    state_backend::{StateError, StateResult},
    worker::{FenceOutcome, WorkerAdmission},
};

pub use shaula_core::worker::recovery::RecoveryRecord;

impl SqliteStateBackend {
    pub async fn recovery_records(&self) -> StateResult<Vec<RecoveryRecord>> {
        let rows = workers::Entity::find()
            .select_only()
            .column(workers::Column::GenerationId)
            .columns([states::Column::WorkerEpoch, states::Column::WorkerAttempt])
            .columns([workers::Column::Phase, workers::Column::ProcessIdentity])
            .column(generations::Column::WorkspacePath)
            .column(states::Column::CreateStarted)
            .expr(col(workers::Column::ProtectedInput).is_not_null())
            .inner_join_on(
                states::Entity,
                same(states::Column::GenerationId, workers::Column::GenerationId),
            )
            .inner_join_on(
                generations::Entity,
                same(generations::Column::Id, workers::Column::GenerationId),
            )
            .filter(workers::Column::Phase.ne("completed"))
            .filter(generations::Column::State.ne("Destroyed"))
            .order_by_asc(generations::Column::CreatedAt)
            .order_by_asc(generations::Column::Id)
            .into_tuple::<(
                String,
                i64,
                String,
                String,
                Option<String>,
                String,
                bool,
                bool,
            )>()
            .all(self.store.connection())
            .await
            .map_err(unavailable)?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    generation_id,
                    worker_epoch,
                    worker_attempt,
                    phase,
                    process_identity,
                    workspace_path,
                    create_started,
                    has_input,
                )| RecoveryRecord {
                    generation_id,
                    worker_epoch,
                    worker_attempt,
                    phase,
                    process_identity,
                    workspace_path,
                    create_started,
                    has_input,
                },
            )
            .collect())
    }

    /// `outcome` comes exclusively from the trusted local Executor (or an
    /// admitted record which never entered launch_pending), never worker IPC.
    pub async fn recover_worker(
        &self,
        record: &RecoveryRecord,
        outcome: FenceOutcome,
        admissions: &WorkerAdmissions,
        preserve_evidence: bool,
    ) -> StateResult<()> {
        let tx = self.writer(&record.generation_id).await?;
        let state = Row::load(&tx, &record.generation_id).await?;
        state.check_claim(&record.claim()?)?;
        let worker = WorkerRow::read(&tx, &record.generation_id)
            .await?
            .ok_or(StateError::Conflict)?;
        if worker.phase != record.phase
            || worker.process_identity != record.process_identity
            || state.sealed
        {
            return Err(StateError::Conflict);
        }
        let fenced = outcome == FenceOutcome::Fenced;
        if worker.phase == "quarantined" {
            // A later reboot/verified fence may make explicit operator finalize
            // safe, but never silently resumes a quarantined Generation.
            if fenced {
                fences::Entity::update_many()
                    .col_expr(fences::Column::ConfirmedFenced, Expr::value(true))
                    .filter(fences::Column::WorkerAttempt.eq(&record.worker_attempt))
                    .filter(fences::Column::GenerationId.eq(&record.generation_id))
                    .filter(fences::Column::WorkerEpoch.eq(record.worker_epoch))
                    .exec(&tx)
                    .await
                    .map_err(unavailable)?;
                release_command(&tx, &record.generation_id).await?;
            }
            return tx.commit().await.map_err(unavailable);
        }
        let lock = state.lock_bytes(&tx).await?;
        // Resource cleanup may finish before CI registration removal. Its
        // exact empty-state proof survives a fenced restart while that removal
        // is retried, provided no later command, lock or revision superseded it.
        let cleanup_revision = worker.cleanup_revision.filter(|revision| {
            *revision == state.revision
                && worker.handover.is_none()
                && worker.active_command.is_none()
                && lock.is_none()
        });
        fences::Entity::insert(fences::ActiveModel {
            worker_attempt: Set(record.worker_attempt.clone()),
            generation_id: Set(record.generation_id.clone()),
            worker_epoch: Set(record.worker_epoch),
            process_identity: Set(record.process_identity.clone()),
            orphan_lock: Set(lock),
            outcome: Set(if fenced { "fenced" } else { "unknown" }.into()),
            ..Default::default()
        })
        .exec_without_returning(&tx)
        .await
        .map_err(unavailable)?;
        // Only a cleanup this recovery would otherwise admit may anchor a
        // failed Create: the fence proves no writer of the state remains.
        let anchorable =
            fenced && !preserve_evidence && (!record.create_started || record.has_input);
        let complete_state = match state.snapshot(&tx).await {
            Ok(Some(snapshot)) => {
                !state.create_started
                    || super::create_proof::verified(
                        &tx,
                        &record.generation_id,
                        &snapshot.document,
                        anchorable,
                    )
                    .await?
            }
            _ => false,
        };
        if !fenced
            || preserve_evidence
            || (record.create_started && !record.has_input)
            || !complete_state
        {
            states::Entity::update_many()
                .col_expr(states::Column::Revoked, Expr::value(true))
                .filter(states::Column::GenerationId.eq(&record.generation_id))
                .exec(&tx)
                .await
                .map_err(unavailable)?;
            set_worker(
                &tx,
                &record.generation_id,
                [(workers::Column::Phase, Expr::value("quarantined"))],
            )
            .await?;
            generations::Entity::update_many()
                .col_expr(generations::Column::State, Expr::value("Quarantined"))
                .filter(generations::Column::Id.eq(&record.generation_id))
                .exec(&tx)
                .await
                .map_err(unavailable)?;
            if fenced {
                release_command(&tx, &record.generation_id).await?;
            }
            return tx.commit().await.map_err(unavailable);
        }
        let mut admission = WorkerAdmission::new(record.claim()?.generation_id);
        admission.claim.worker_epoch = state
            .worker_epoch
            .checked_add(1)
            .ok_or(StateError::Conflict)?;
        admission.cleanup_only = true;
        // Orphan lock removal and credential rotation are a single commit,
        // after the exact old containment was proved empty and audited above.
        states::Entity::update_many()
            .col_expr(
                states::Column::WorkerEpoch,
                Expr::value(admission.claim.worker_epoch),
            )
            .col_expr(
                states::Column::WorkerAttempt,
                Expr::value(admission.claim.worker_attempt.to_string()),
            )
            .col_expr(
                states::Column::CapabilityHash,
                Expr::value(admission.state.verifier()),
            )
            .col_expr(states::Column::Revoked, Expr::value(false))
            .col_expr(states::Column::LockId, null())
            .col_expr(
                states::Column::LockInfo,
                Expr::value(Option::<Vec<u8>>::None),
            )
            .filter(states::Column::GenerationId.eq(&record.generation_id))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        set_worker(
            &tx,
            &record.generation_id,
            [
                (
                    workers::Column::ControlHash,
                    Expr::value(admission.control.verifier()),
                ),
                (workers::Column::Phase, Expr::value("admitted")),
                (workers::Column::ProcessIdentity, null()),
                (workers::Column::CleanupOnly, Expr::value(true)),
                (workers::Column::Handover, null()),
                (workers::Column::ActiveCommand, null()),
                (
                    workers::Column::CleanupRevision,
                    Expr::value(cleanup_revision),
                ),
            ],
        )
        .await?;
        // Recovery never re-enters Create/JIT. Existing CI removal gates still
        // decide when the cleanup-only replacement may actually destroy.
        generations::Entity::update_many()
            .col_expr(generations::Column::State, Expr::value("CleanupRequired"))
            .filter(generations::Column::Id.eq(&record.generation_id))
            .filter(generations::Column::State.is_in([
                "CreatePending",
                "Creating",
                "WaitingOnline",
            ]))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        admissions.publish(admission)
    }

    pub(crate) async fn recovered_state_identity(
        &self,
        id: &str,
    ) -> StateResult<Option<(String, u64)>> {
        let Some(worker) = WorkerRow::read(self.store.connection(), id).await? else {
            return Ok(None);
        };
        if !worker.cleanup_only || worker.phase == "quarantined" {
            return Ok(None);
        }
        let row = Row::load(self.store.connection(), id).await?;
        if row.revoked {
            return Err(StateError::Unauthorized);
        }
        let snapshot = row
            .snapshot(self.store.connection())
            .await?
            .ok_or(StateError::Unavailable)?;
        Ok(Some((
            snapshot.document.lineage().into(),
            u64::try_from(snapshot.document.serial()).map_err(|_| StateError::Invalid)?,
        )))
    }
}

/// A fenced executor can hold no command or spawn handover any longer.
async fn release_command(tx: &DatabaseTransaction, id: &str) -> StateResult<()> {
    set_worker(
        tx,
        id,
        [
            (workers::Column::Handover, null()),
            (workers::Column::ActiveCommand, null()),
        ],
    )
    .await
}
