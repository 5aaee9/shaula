use sea_orm::ConnectionTrait;

use super::{sql, TestResult};
use crate::Store;

#[tokio::test]
async fn failed_http_state_migration_rolls_back_schema_and_migration_history() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Store::open(&dir.path().join("migration.db")).await?;
    // Force the last migration to fail after earlier DDL has run. The unrelated
    // pre-existing table must be preserved, and none of the earlier migrations
    // may be falsely marked applied or leave half of the new schema committed.
    store
        .connection()
        .execute_unprepared("CREATE TABLE generation_http_state (unrelated TEXT)")
        .await?;
    assert!(store.migrate().await.is_err());
    let partial = store
        .connection()
        .query_one(sql(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'runner_generations'",
            vec![],
        ))
        .await?;
    assert!(
        partial.is_none(),
        "failed migrations must roll back their DDL"
    );
    let history = store
        .connection()
        .query_one(sql(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'seaql_migrations'",
            vec![],
        ))
        .await?;
    assert!(
        history.is_none(),
        "schema and migration history are one transaction"
    );
    // Remove only our injected obstruction, then retry the same migration path.
    store
        .connection()
        .execute_unprepared("DROP TABLE generation_http_state")
        .await?;
    store.migrate().await?;
    store.migrate().await?;
    Ok(())
}
