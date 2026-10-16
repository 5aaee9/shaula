//! Transactional, Generation-scoped Terraform state. This is NOT a worker
//! scheduler: no lease expiry, force unlock, state purge or implicit migration.

mod admin;
mod admissions;
mod cleanup;
mod create_proof;
pub use cleanup::CompletedWorkspace;
mod cutover;
mod legacy;
mod operations;
mod operator;
mod recovery;
mod row;
mod worker;
mod worker_completion;
mod worker_effect;
pub use recovery::RecoveryRecord;

pub use cutover::MigrationClassification;
pub use legacy::LegacyGeneration;
mod worker_journal;

use sea_orm::{
    sea_query::{Expr, SimpleExpr},
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter,
};
use shaula_core::state_backend::{StateError, StateResult};

use crate::entities::http_state::{
    generation_http_state as states, lifecycle_deployment as deployment,
    lifecycle_fences as fences, lifecycle_workers as workers,
};
use crate::query_ext::col;
use crate::Store;

pub use admissions::WorkerAdmissions;
pub use worker_journal::SqliteWorkerJournal;

/// Use only behind the private listener. Administrative methods are local
/// daemon calls, never worker HTTP routes.
#[derive(Clone)]
pub struct SqliteStateBackend {
    store: Store,
}

impl SqliteStateBackend {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// SQLite deferred transactions must acquire the writer BEFORE any
    /// ownership/version reads. This first write (even for a missing key)
    /// reserves SQLite's single writer, like BEGIN IMMEDIATE. It works across
    /// pools/Store handles and rolls back safely on cancellation; a Rust mutex
    /// alone would not. No transaction is held across HTTP body reads.
    async fn writer(&self, generation_id: &str) -> StateResult<DatabaseTransaction> {
        let tx = self
            .store
            .begin()
            .await
            .map_err(|_| StateError::Unavailable)?;
        states::Entity::update_many()
            .col_expr(
                states::Column::Revision,
                col(states::Column::Revision).into(),
            )
            .filter(states::Column::GenerationId.eq(generation_id))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        Ok(tx)
    }
}

fn unavailable(_: sea_orm::DbErr) -> StateError {
    StateError::Unavailable
}

fn exactly_one(count: u64) -> StateResult<()> {
    if count == 1 {
        Ok(())
    } else {
        Err(StateError::Conflict)
    }
}

pub(super) fn null() -> SimpleExpr {
    Expr::value(Option::<String>::None)
}

/// Unconditional column writes to one worker row; callers hold the writer.
pub(super) async fn set_worker<const N: usize>(
    tx: &DatabaseTransaction,
    id: &str,
    changes: [(workers::Column, SimpleExpr); N],
) -> StateResult<()> {
    changes
        .into_iter()
        .fold(workers::Entity::update_many(), |update, (column, value)| {
            update.col_expr(column, value)
        })
        .filter(workers::Column::GenerationId.eq(id))
        .exec(tx)
        .await
        .map_err(unavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests;

mod recovery_journal;
