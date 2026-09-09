//! Read models and retained evidence. No lifecycle decisions originate here.
mod association;
mod ingest;
mod query;
mod read;
mod retention;
mod scope;

use sea_orm::{ConnectionTrait, DbBackend, QueryResult, Statement, Value};
use serde::{de::DeserializeOwned, Serialize};

use crate::{StoreError, StoreResult};

async fn rows<C: ConnectionTrait>(
    db: &C,
    sql: &str,
    values: Vec<Value>,
) -> StoreResult<Vec<QueryResult>> {
    Ok(db
        .query_all(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            sql,
            values,
        ))
        .await?)
}

async fn execute<C: ConnectionTrait>(db: &C, sql: &str, values: Vec<Value>) -> StoreResult<()> {
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        sql,
        values,
    ))
    .await?;
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> StoreResult<String> {
    serde_json::to_string(value)
        .map_err(|_| StoreError::Corrupt("job metadata cannot be encoded".into()))
}

fn decode<T: DeserializeOwned>(json: &str) -> StoreResult<T> {
    serde_json::from_str(json)
        .map_err(|_| StoreError::Corrupt("job metadata cannot be decoded".into()))
}

#[cfg(test)]
mod tests;
