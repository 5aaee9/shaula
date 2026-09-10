//! Store handle: single-writer SQLite connection with explicit pragmas.

use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr, TransactionTrait};
use std::time::Duration;

use shaula_store_migration as migration;

/// Typed store error; ORM/SQL details stay sanitized behind a summary.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database unavailable: {0}")]
    Unavailable(String),
    #[error("conflicting concurrent write on {resource}")]
    Conflict { resource: String },
    #[error("policy denied: {reason}")]
    PolicyDenied { reason: &'static str },
    #[error("{0}")]
    Corrupt(String),
}

impl From<DbErr> for StoreError {
    fn from(value: DbErr) -> Self {
        StoreError::Unavailable(value.to_string())
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// The store handle. One daemon owns one database; the ownership lock lives
/// above this layer in the binary.
#[derive(Clone)]
pub struct Store {
    db: DatabaseConnection,
}

impl Store {
    /// Opens (creating if needed) the SQLite database with WAL, foreign
    /// keys and a bounded busy timeout.
    pub async fn open(path: &std::path::Path) -> StoreResult<Self> {
        let url = format!(
            "sqlite://{}?mode=rwc",
            path.to_string_lossy().replace('\\', "/")
        );
        let mut options = ConnectOptions::new(url);
        options
            .max_connections(8)
            .min_connections(1)
            .connect_timeout(Duration::from_secs(10))
            // State, lock metadata and capability verifiers must not appear
            // in SQL diagnostics, even under a debug-level subscriber.
            .sqlx_logging(false)
            // Apply on EVERY pooled connection, not four arbitrary checkouts.
            // FULL preserves acknowledged authoritative state across a crash.
            // The busy timeout is the write-queue budget: SQLite serializes
            // writers, and under concurrent creates every commit fsyncs the
            // WAL (FULL) while checkpoints contend for the same lock — the
            // 2026-09-10 storm (8 parallel matrix generations) exhausted a
            // 5s budget and failed ticks. 60s lets short transactions queue
            // instead of failing; no transaction spans a network call, so
            // waiting cannot deadlock (spec 0010 §8.1).
            .map_sqlx_sqlite_opts(|options| {
                options
                    .pragma("journal_mode", "WAL")
                    .pragma("synchronous", "FULL")
                    .foreign_keys(true)
                    .busy_timeout(Duration::from_secs(60))
            });
        let db = Database::connect(options).await?;
        Ok(Self { db })
    }

    /// Connection accessor for migrations only.
    pub(crate) fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Applies pending forward migrations and verifies the durable-format
    /// marker. A LEGACY or missing marker means the database was written
    /// by an unsupported pre-release build: the daemon fails closed
    /// instead of resolving durable identities under a different encoding
    /// than the one that wrote them (R8-03) — rebuild the data directory.
    pub async fn migrate(&self) -> StoreResult<()> {
        migration::migrate(&self.db).await?;
        self.check_durable_format().await
    }

    async fn check_durable_format(&self) -> StoreResult<()> {
        use sea_orm::{ConnectionTrait, Statement};
        let row = self
            .db
            .query_one(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT version FROM durable_format",
            ))
            .await
            .map_err(StoreError::from)?;
        let version: Option<i64> = row
            .map(|r| r.try_get::<i64>("", "version"))
            .transpose()
            .map_err(StoreError::from)?;
        match version {
            Some(v) if v == migration::m0008_auth_multi_account::DURABLE_FORMAT_VERSION_V2 => {
                Ok(())
            }
            Some(v) if v == migration::m0005_durable_format::DURABLE_FORMAT_LEGACY => {
                Err(StoreError::Corrupt(
                    "data directory was written by an unsupported pre-release build (legacy durable identity encoding); no compatibility path exists — rebuild the data directory"
                        .to_string(),
                ))
            }
            Some(v) if v < migration::m0008_auth_multi_account::DURABLE_FORMAT_VERSION_V2 => Err(
                StoreError::Corrupt(
                    "data directory predates the multi-account auth format and did not migrate; upgrade through a build that migrates it"
                        .to_string(),
                ),
            ),
            Some(v) => Err(StoreError::Corrupt(format!(
                "data directory durable-format version {v} is newer than this build ({}); upgrade the binary",
                migration::m0008_auth_multi_account::DURABLE_FORMAT_VERSION_V2
            ))),            None => Err(StoreError::Corrupt(
                "data directory has no durable-format marker; rebuild the data directory"
                    .to_string(),
            )),
        }
    }

    /// Begins a short transaction with its SQLite writer reserved before any
    /// authority snapshot is read. Transactions never span network calls or
    /// subprocesses; callers must keep them small and must not nest them.
    pub(crate) async fn begin(&self) -> StoreResult<sea_orm::DatabaseTransaction> {
        use sea_orm::ConnectionTrait;
        let tx = self.db.begin().await?;
        // SeaORM 1.1 always starts SQLite transactions with BEGIN DEFERRED;
        // isolation/access-mode options do not select BEGIN IMMEDIATE. A
        // write statement with no matching rows obtains that same writer
        // reservation without changing the format marker or any other data.
        // Acquire it BEFORE a SELECT, so concurrent listener/reconcile commits
        // wait at this boundary instead of failing a stale snapshot upgrade
        // with SQLITE_BUSY_SNAPSHOT (517). Even read-only transactional
        // snapshots use this short, serialized boundary; ordinary read queries
        // remain concurrent on the WAL pool. SeaORM still owns commit/rollback
        // and cancellation, including when reservation is interrupted.
        tx.execute_unprepared("UPDATE durable_format SET version=version WHERE 0")
            .await?;
        Ok(tx)
    }
}
