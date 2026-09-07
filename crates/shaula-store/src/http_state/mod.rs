//! Transactional, Generation-scoped Terraform state. This is NOT a worker
//! scheduler: no lease expiry, force unlock, state purge or implicit migration.

mod admin;
mod operations;
mod row;

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement, Value};
use shaula_core::state_backend::{StateError, StateResult};

use crate::Store;

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
        tx.execute(sql(
            "UPDATE generation_http_state SET revision = revision WHERE generation_id = ?",
            vec![generation_id.into()],
        ))
        .await
        .map_err(unavailable)?;
        Ok(tx)
    }
}

fn sql(query: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, query, values)
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

#[cfg(test)]
mod tests;
