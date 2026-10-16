//! File-backed sanitized diagnostics; no lifecycle mutation is permitted here.
mod files;
mod invocations;
mod prune;
mod reads;
mod writes;

use sea_orm::{
    sea_query::{Expr, Func, SimpleExpr, SubQueryStatement},
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
};
use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    operation_log::{Invocation, LogConfig},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

use crate::entities::operation_logs::{
    operation_log_chunks as log_chunks, operation_log_invocations as log_invocations,
};
use crate::Store;

#[derive(Clone)]
pub struct OperationLogArchive {
    store: Store,
    root: PathBuf,
    config: LogConfig,
    writer: Arc<Mutex<()>>,
    // Conservative reservations include failed/partial publications until restart.
    disk_usage: Arc<AtomicU64>,
}

pub(super) fn unavailable() -> CoreError {
    CoreError::new(
        ReasonCode::StorageUnavailable,
        "operation log storage unavailable",
    )
}

pub(super) fn invalid() -> CoreError {
    CoreError::new(ReasonCode::SpecInvalid, "invalid operation log request")
}

pub(super) fn missing() -> CoreError {
    CoreError::new(
        ReasonCode::TargetHiddenOrNotFound,
        "operation log not found",
    )
}

pub(super) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `COALESCE(SUM(bytes),0)` over the selected chunk rows.
fn retained_bytes() -> SimpleExpr {
    Func::coalesce([
        Expr::col(log_chunks::Column::Bytes).sum(),
        Expr::val(0).into(),
    ])
    .into()
}

impl OperationLogArchive {
    pub async fn open(store: Store, root: PathBuf, config: LogConfig) -> CoreResult<Self> {
        config.validate().map_err(|_| invalid())?;
        files::private_directory(&root).await?;
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|_| unavailable())?;
        let archive = Self {
            store,
            root,
            config,
            writer: Arc::new(Mutex::new(())),
            disk_usage: Arc::new(AtomicU64::new(0)),
        };
        archive.recover().await?;
        Ok(archive)
    }

    pub(super) async fn load(&self, id: &str) -> CoreResult<Invocation> {
        uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
        let json = log_invocations::Entity::find_by_id(id)
            .select_only()
            .column(log_invocations::Column::RecordJson)
            .into_tuple::<String>()
            .one(self.store.connection())
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(missing)?;
        let mut record: Invocation = serde_json::from_str(&json).map_err(|_| unavailable())?;
        let total = log_chunks::Entity::find()
            .select_only()
            .expr(retained_bytes())
            .filter(log_chunks::Column::InvocationId.eq(id))
            .into_tuple::<i64>()
            .one(self.store.connection())
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        record.retained_bytes = u64::try_from(total).map_err(|_| unavailable())?;
        Ok(record)
    }

    pub(super) async fn save(&self, record: &Invocation) -> CoreResult<()> {
        let json = serde_json::to_string(record).map_err(|_| unavailable())?;
        let retained = i64::try_from(record.retained_bytes).map_err(|_| invalid())?;
        log_invocations::Entity::update_many()
            .col_expr(log_invocations::Column::RecordJson, Expr::value(json))
            .col_expr(
                log_invocations::Column::SealedAt,
                Expr::value(record.capture_sealed_at),
            )
            .col_expr(
                log_invocations::Column::RetainedBytes,
                Expr::value(retained),
            )
            .filter(log_invocations::Column::Id.eq(record.id.as_str()))
            .exec(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        Ok(())
    }

    /// Called only while the daemon ownership lock is held and before new log writers.
    async fn recover(&self) -> CoreResult<()> {
        let per_invocation = log_chunks::Entity::find()
            .select_only()
            .expr(retained_bytes())
            .filter(
                Expr::col((log_chunks::Entity, log_chunks::Column::InvocationId))
                    .equals((log_invocations::Entity, log_invocations::Column::Id)),
            )
            .into_query();
        log_invocations::Entity::update_many()
            .col_expr(
                log_invocations::Column::RetainedBytes,
                SimpleExpr::SubQuery(
                    None,
                    Box::new(SubQueryStatement::SelectStatement(per_invocation)),
                ),
            )
            .exec(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        let rows = log_invocations::Entity::find()
            .select_only()
            .column(log_invocations::Column::RecordJson)
            .filter(log_invocations::Column::SealedAt.is_null())
            .into_tuple::<String>()
            .all(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        for json in rows {
            let mut record: Invocation = serde_json::from_str(&json).map_err(|_| unavailable())?;
            record.retained_bytes = self.load(&record.id).await?.retained_bytes;
            record.capture_status = "partial".into();
            record.execution_outcome = "unknown".into();
            record.reason = Some("writer_interrupted".into());
            record.capture_sealed_at = Some(now());
            self.save(&record).await?;
        }
        self.reap_orphans().await?;
        self.disk_usage
            .store(self.disk_bytes().await?, Ordering::Relaxed);
        self.prune_locked(0).await
    }

    pub async fn maintenance(&self) -> CoreResult<()> {
        let _writer = self.writer.lock().await;
        self.reap_orphans().await?;
        // A timed-out filesystem future can still have an in-flight blocking write.
        // Never release its reservation using a scan that raced that write.
        self.disk_usage
            .fetch_max(self.disk_bytes().await?, Ordering::Relaxed);
        self.prune_locked(0).await?;
        self.prune_metadata().await
    }

    async fn prune_metadata(&self) -> CoreResult<()> {
        let cutoff =
            now().saturating_sub(i64::from(self.config.metadata_retention_days) * 86_400_000);
        let db = self.store.connection();
        db.execute(db.get_database_backend().build(&prune::metadata(cutoff)))
            .await
            .map_err(|_| unavailable())?;
        Ok(())
    }

    async fn prune_locked(&self, incoming: u64) -> CoreResult<()> {
        let cutoff = now().saturating_sub(i64::from(self.config.retention_days) * 86_400_000);
        let rows = log_invocations::Entity::find()
            .select_only()
            .columns([
                log_invocations::Column::Id,
                log_invocations::Column::SealedAt,
            ])
            .filter(log_invocations::Column::SealedAt.is_not_null())
            .filter(log_invocations::Column::RetainedBytes.gt(0))
            .order_by_asc(log_invocations::Column::SealedAt)
            .order_by_asc(log_invocations::Column::Id)
            .into_tuple::<(String, i64)>()
            .all(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        let mut used = self.disk_usage.load(Ordering::Relaxed);
        for (id, sealed) in rows {
            if sealed > cutoff && used.saturating_add(incoming) <= self.config.quota_bytes {
                continue;
            }
            let mut record = self.load(&id).await?;
            let chunks = self.chunk_rows(&id).await?;
            for chunk in chunks {
                self.remove_chunk(&id, &chunk).await?;
            }
            used = self.disk_usage.load(Ordering::Relaxed);
            record.retained_bytes = 0;
            record.capture_status = "expired".into();
            record.reason = Some(
                if sealed <= cutoff {
                    "retention_expired"
                } else {
                    "quota_evicted"
                }
                .into(),
            );
            self.save(&record).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod paging_tests;
