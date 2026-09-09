//! File-backed sanitized diagnostics; no lifecycle mutation is permitted here.
mod files;
mod invocations;
mod reads;
mod writes;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
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
        let row = self
            .store
            .connection()
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT record_json FROM operation_log_invocations WHERE id=?",
                [id.into()],
            ))
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(missing)?;
        let json: String = row.try_get("", "record_json").map_err(|_| unavailable())?;
        let mut record: Invocation = serde_json::from_str(&json).map_err(|_| unavailable())?;
        let total = self.store.connection().query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT COALESCE(SUM(bytes),0) AS bytes FROM operation_log_chunks WHERE invocation_id=?", [id.into()]
        )).await.map_err(|_| unavailable())?.ok_or_else(unavailable)?;
        record.retained_bytes = u64::try_from(
            total
                .try_get::<i64>("", "bytes")
                .map_err(|_| unavailable())?,
        )
        .map_err(|_| unavailable())?;
        Ok(record)
    }

    pub(super) async fn save(&self, record: &Invocation) -> CoreResult<()> {
        let json = serde_json::to_string(record).map_err(|_| unavailable())?;
        self.store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "UPDATE operation_log_invocations SET record_json=?,sealed_at=?,retained_bytes=? WHERE id=?",
            [json.into(), record.capture_sealed_at.into(), i64::try_from(record.retained_bytes).map_err(|_| invalid())?.into(), record.id.clone().into()]
        )).await.map_err(|_| unavailable())?;
        Ok(())
    }

    /// Called only while the daemon ownership lock is held and before new log writers.
    async fn recover(&self) -> CoreResult<()> {
        self.store.connection().execute_unprepared("UPDATE operation_log_invocations SET retained_bytes=(SELECT COALESCE(SUM(bytes),0) FROM operation_log_chunks WHERE invocation_id=operation_log_invocations.id)").await.map_err(|_| unavailable())?;
        let rows = self
            .store
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT record_json FROM operation_log_invocations WHERE sealed_at IS NULL",
            ))
            .await
            .map_err(|_| unavailable())?;
        for row in rows {
            let json: String = row.try_get("", "record_json").map_err(|_| unavailable())?;
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
        self.store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "DELETE FROM operation_log_invocations WHERE sealed_at<? AND retained_bytes=0 AND NOT EXISTS (SELECT 1 FROM runner_generations g WHERE g.id=operation_log_invocations.generation_id AND (g.state!='Destroyed' OR g.updated_at>=?)) AND NOT EXISTS (SELECT 1 FROM workflow_job_observations o JOIN workflow_jobs j ON j.id=o.job_record_id LEFT JOIN workflow_generation_identity i ON i.generation_id=operation_log_invocations.generation_id WHERE j.updated_at>=? AND (json_extract(o.data_json,'$.generation_id')=operation_log_invocations.generation_id OR (i.scope_key=o.scope_key AND i.github_runner_id=o.runner_id)))",
            [cutoff.into(),cutoff.into(),cutoff.into()]
        )).await.map_err(|_| unavailable())?;
        Ok(())
    }

    async fn prune_locked(&self, incoming: u64) -> CoreResult<()> {
        let cutoff = now().saturating_sub(i64::from(self.config.retention_days) * 86_400_000);
        let rows = self.store.connection().query_all(Statement::from_string(DbBackend::Sqlite,
            "SELECT id,sealed_at,retained_bytes FROM operation_log_invocations WHERE sealed_at IS NOT NULL AND retained_bytes>0 ORDER BY sealed_at,id"
        )).await.map_err(|_| unavailable())?;
        let mut used = self.disk_usage.load(Ordering::Relaxed);
        for row in rows {
            let sealed: i64 = row.try_get("", "sealed_at").map_err(|_| unavailable())?;
            if sealed > cutoff && used.saturating_add(incoming) <= self.config.quota_bytes {
                continue;
            }
            let id: String = row.try_get("", "id").map_err(|_| unavailable())?;
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
