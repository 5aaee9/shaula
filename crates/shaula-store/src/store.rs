//! Store handle: single-writer SQLite connection with explicit pragmas.

use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr, TransactionTrait};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    writer_waits: Arc<WriterContentionCounters>,
}

/// Aggregated evidence of SQLite writer contention since the store opened.
///
/// A wait is recorded only when reserving the writer takes at least 10 ms;
/// short scheduler noise is not useful evidence of a lock queue. The counters
/// are intentionally process-local: durable state remains the source of truth,
/// while this snapshot explains transient busy delays in readiness diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriterContention {
    pub waits: u64,
    pub total_wait_ms: u64,
    pub max_wait_ms: u64,
}

#[derive(Default)]
struct WriterContentionCounters {
    waits: AtomicU64,
    total_wait_ms: AtomicU64,
    max_wait_ms: AtomicU64,
}

const CONTENTION_RECORD_AFTER: Duration = Duration::from_millis(10);

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
        Ok(Self {
            db,
            writer_waits: Arc::new(WriterContentionCounters::default()),
        })
    }

    /// Connection accessor for migrations only.
    pub(crate) fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Returns process-local evidence of waits for SQLite's single writer.
    ///
    /// This is diagnostic only; it does not participate in admission or
    /// recovery decisions and resets when the daemon restarts.
    pub fn writer_contention(&self) -> WriterContention {
        WriterContention {
            waits: self.writer_waits.waits.load(Ordering::Relaxed),
            total_wait_ms: self.writer_waits.total_wait_ms.load(Ordering::Relaxed),
            max_wait_ms: self.writer_waits.max_wait_ms.load(Ordering::Relaxed),
        }
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
        let started = Instant::now();
        tx.execute_unprepared("UPDATE durable_format SET version=version WHERE 0")
            .await?;
        let waited = started.elapsed();
        if waited >= CONTENTION_RECORD_AFTER {
            let wait_ms = waited.as_millis().min(u128::from(u64::MAX)) as u64;
            self.writer_waits.waits.fetch_add(1, Ordering::Relaxed);
            self.writer_waits
                .total_wait_ms
                .fetch_add(wait_ms, Ordering::Relaxed);
            let mut previous = self.writer_waits.max_wait_ms.load(Ordering::Relaxed);
            while previous < wait_ms {
                match self.writer_waits.max_wait_ms.compare_exchange_weak(
                    previous,
                    wait_ms,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(observed) => previous = observed,
                }
            }
        }
        Ok(tx)
    }
}
