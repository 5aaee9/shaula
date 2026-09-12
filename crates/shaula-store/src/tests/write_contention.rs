//! A read-then-write transaction reserves its writer before taking a snapshot.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn value(tx: &sea_orm::DatabaseTransaction) -> Result<i64, crate::StoreError> {
    Ok(tx
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT value FROM writer_probe",
        ))
        .await?
        .ok_or_else(|| crate::StoreError::Corrupt("probe missing".into()))?
        .try_get("", "value")?)
}

#[tokio::test]
async fn concurrent_read_then_write_waits_before_snapshot_and_preserves_both_updates() -> TestResult
{
    let dir = tempfile::tempdir()?;
    let store = crate::Store::open(&dir.path().join("contention.db")).await?;
    store.migrate().await?;
    store
        .connection()
        .execute_unprepared(
            "CREATE TABLE writer_probe(value INTEGER NOT NULL); INSERT INTO writer_probe VALUES(0)",
        )
        .await?;
    let first = store.begin().await?;
    assert_eq!(value(&first).await?, 0);
    let competing = async {
        let second = store.begin().await?;
        let before = value(&second).await?;
        second
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE writer_probe SET value=?",
                [(before + 1).into()],
            ))
            .await?;
        second.commit().await?;
        Ok::<_, crate::StoreError>(before)
    };
    tokio::pin!(competing);
    let early = tokio::time::timeout(Duration::from_millis(100), &mut competing).await;
    // With BEGIN DEFERRED, the competitor has already committed here and this
    // upgrade fails deterministically with SQLITE_BUSY_SNAPSHOT (517).
    first
        .execute_unprepared("UPDATE writer_probe SET value=1")
        .await?;
    first.commit().await?;
    assert!(
        early.is_err(),
        "the second writer must wait before reading its head"
    );
    assert_eq!(
        competing.await?,
        1,
        "the resumed writer observes the committed head"
    );
    let final_read = store.begin().await?;
    assert_eq!(value(&final_read).await?, 2);
    final_read.commit().await?;
    Ok(())
}

#[tokio::test]
async fn cancelling_a_waiting_transaction_releases_the_connection_and_writer() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = crate::Store::open(&dir.path().join("cancelled.db")).await?;
    store.migrate().await?;
    let first = store.begin().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), store.begin())
            .await
            .is_err()
    );
    first.rollback().await?;
    let subsequent = tokio::time::timeout(Duration::from_secs(2), store.begin()).await??;
    subsequent.commit().await?;
    Ok(())
}

#[tokio::test]
async fn writer_contention_snapshot_records_a_waiting_writer() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = crate::Store::open(&dir.path().join("observed-contention.db")).await?;
    store.migrate().await?;

    let first = store.begin().await?;
    let competing = async {
        let second = store.begin().await?;
        second.commit().await?;
        Ok::<_, crate::StoreError>(())
    };
    tokio::pin!(competing);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut competing)
            .await
            .is_err(),
        "the competing writer must remain queued while the first writer is held"
    );
    first.rollback().await?;
    competing.await?;

    let contention = store.writer_contention();
    assert!(
        contention.waits >= 1,
        "the queued writer wait must be recorded"
    );
    assert!(
        contention.max_wait_ms >= 50,
        "the recorded wait should cover the held writer interval"
    );
    assert!(contention.total_wait_ms >= contention.max_wait_ms);
    Ok(())
}
