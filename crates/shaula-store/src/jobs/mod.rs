//! Read models and retained evidence. No lifecycle decisions originate here.
mod association;
mod forgejo;
mod forgejo_projection;
mod forgejo_read;
#[cfg(test)]
mod forgejo_result_tests;
mod forgejo_results;
mod forgejo_scope;
#[cfg(test)]
mod forgejo_tests;
mod gitlab;
mod gitlab_projection;
mod gitlab_read;
mod gitlab_results;
mod gitlab_scope;
mod ingest;
mod query;
mod read;
mod retention;
mod scope;
mod views;

use sea_orm::{
    sea_query::{SelectStatement, SimpleExpr},
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryResult, QuerySelect,
};
use serde::{de::DeserializeOwned, Serialize};
use shaula_core::registry::FleetRuntimeGuard;

use crate::entities::{
    fleet::{fleet_revisions, fleets},
    jobs::{workflow_generation_identity as identity, workflow_job_observations as observations},
};
use crate::query_ext::{col, json_extract, same, JoinOn};
use crate::{StoreError, StoreResult};

/// Runs a builder-made select whose rows are read by column name.
async fn select_rows<C: ConnectionTrait>(
    db: &C,
    query: &SelectStatement,
) -> StoreResult<Vec<QueryResult>> {
    Ok(db.query_all(db.get_database_backend().build(query)).await?)
}

/// Literal-SQL fixture inspection for this module's tests only.
#[cfg(test)]
async fn rows<C: ConnectionTrait>(
    db: &C,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> StoreResult<Vec<QueryResult>> {
    let statement =
        sea_orm::Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, sql, values);
    Ok(db.query_all(statement).await?)
}

/// The desired revision's spec, only while `guard` still names the live,
/// unfenced Fleet incarnation.
async fn guarded_spec<C: ConnectionTrait>(
    db: &C,
    fleet: &str,
    guard: &FleetRuntimeGuard,
) -> StoreResult<Option<String>> {
    Ok(fleets::Entity::find()
        .select_only()
        .column(fleet_revisions::Column::SpecJson)
        .inner_join_on(
            fleet_revisions::Entity,
            same(fleet_revisions::Column::FleetKey, fleets::Column::Key)
                .and(same(
                    fleet_revisions::Column::Incarnation,
                    fleets::Column::Incarnation,
                ))
                .and(same(
                    fleet_revisions::Column::Revision,
                    fleets::Column::DesiredRevision,
                )),
        )
        .filter(fleets::Column::Key.eq(fleet))
        .filter(fleets::Column::Incarnation.eq(guard.incarnation.as_str()))
        .filter(fleets::Column::DesiredRevision.eq(guard.desired_revision))
        .filter(fleets::Column::MutationFence.eq(guard.mutation_fence))
        .filter(fleets::Column::DeletionMarker.eq(false))
        .filter(fleets::Column::Tombstone.eq(false))
        .into_tuple::<String>()
        .one(db)
        .await?)
}

/// The GitHub observation's own record names the identity's Generation, or
/// both carry the same positive runner ID within the joined scope.
fn github_generation_evidence() -> SimpleExpr {
    col(identity::Column::GenerationId)
        .eq(json_extract(
            observations::Column::DataJson.as_column_ref(),
            "$.generation_id",
        ))
        .or(col(observations::Column::RunnerId).gt(0).and(same(
            identity::Column::GithubRunnerId,
            observations::Column::RunnerId,
        )))
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
