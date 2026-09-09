use super::{invalid, unavailable, OperationLogArchive};
use sea_orm::{ConnectionTrait, DbBackend, QueryResult, Statement};
use sha2::{Digest, Sha256};
use shaula_core::error::CoreResult;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tokio::io::AsyncWriteExt;

pub(super) async fn private_directory(path: &Path) -> CoreResult<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => return Err(invalid()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = tokio::fs::DirBuilder::new();
            builder.recursive(false);
            #[cfg(unix)]
            builder.mode(0o700);
            builder.create(path).await.map_err(|_| unavailable())?;
        }
        Err(_) => return Err(unavailable()),
    }
    Ok(())
}

impl OperationLogArchive {
    pub(super) async fn directory(&self, id: &str) -> CoreResult<PathBuf> {
        let parsed = uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
        if parsed.to_string() != id {
            return Err(invalid());
        }
        let path = self.root.join(id);
        private_directory(&path).await?;
        Ok(path)
    }

    pub(super) async fn publish(&self, id: &str, sequence: u64, text: &str) -> CoreResult<String> {
        let dir = self.directory(id).await?;
        let name = format!("{sequence:020}.log");
        let final_path = dir.join(&name);
        if tokio::fs::symlink_metadata(&final_path).await.is_ok() {
            return Err(invalid());
        }
        let temporary = dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let mut options = tokio::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).await.map_err(|_| unavailable())?;
        file.write_all(text.as_bytes())
            .await
            .map_err(|_| unavailable())?;
        file.sync_all().await.map_err(|_| unavailable())?;
        drop(file);
        tokio::fs::rename(temporary, &final_path)
            .await
            .map_err(|_| unavailable())?;
        #[cfg(unix)]
        {
            tokio::fs::File::open(&dir)
                .await
                .map_err(|_| unavailable())?
                .sync_all()
                .await
                .map_err(|_| unavailable())?;
        }
        Ok(name)
    }

    pub(super) async fn chunk_rows(&self, id: &str) -> CoreResult<Vec<QueryResult>> {
        self.store
            .connection()
            .query_all(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT * FROM operation_log_chunks WHERE invocation_id=? ORDER BY sequence",
                [id.into()],
            ))
            .await
            .map_err(|_| unavailable())
    }

    pub(super) async fn chunk_text(&self, id: &str, row: &QueryResult) -> CoreResult<String> {
        let sequence: i64 = row.try_get("", "sequence").map_err(|_| unavailable())?;
        let path = self
            .directory(id)
            .await?
            .join(format!("{sequence:020}.log"));
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| unavailable())?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 65536 {
            return Err(unavailable());
        }
        let bytes = tokio::fs::read(path).await.map_err(|_| unavailable())?;
        let digest: String = row.try_get("", "digest").map_err(|_| unavailable())?;
        if hex::encode(Sha256::digest(&bytes)) != digest {
            return Err(unavailable());
        }
        String::from_utf8(bytes).map_err(|_| unavailable())
    }

    pub(super) async fn remove_chunk(&self, id: &str, row: &QueryResult) -> CoreResult<()> {
        let sequence: i64 = row.try_get("", "sequence").map_err(|_| unavailable())?;
        let path = self
            .directory(id)
            .await?
            .join(format!("{sequence:020}.log"));
        match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|_| unavailable())?;
                self.disk_usage
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                        Some(used.saturating_sub(meta.len()))
                    })
                    .map_err(|_| unavailable())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(unavailable()),
        }
        self.store
            .connection()
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "DELETE FROM operation_log_chunks WHERE invocation_id=? AND sequence=?",
                [id.into(), sequence.into()],
            ))
            .await
            .map_err(|_| unavailable())?;
        Ok(())
    }

    pub(super) async fn disk_bytes(&self) -> CoreResult<u64> {
        let mut used = 0u64;
        let mut directories = tokio::fs::read_dir(&self.root)
            .await
            .map_err(|_| unavailable())?;
        while let Some(dir) = directories.next_entry().await.map_err(|_| unavailable())? {
            let meta = dir.metadata().await.map_err(|_| unavailable())?;
            if !dir.file_type().await.map_err(|_| unavailable())?.is_dir() {
                return Err(unavailable());
            }
            if !meta.is_dir() {
                return Err(unavailable());
            }
            let mut files = tokio::fs::read_dir(dir.path())
                .await
                .map_err(|_| unavailable())?;
            while let Some(file) = files.next_entry().await.map_err(|_| unavailable())? {
                if !file.file_type().await.map_err(|_| unavailable())?.is_file() {
                    return Err(unavailable());
                }
                used = used.saturating_add(file.metadata().await.map_err(|_| unavailable())?.len());
            }
        }
        Ok(used)
    }

    pub(super) async fn reap_orphans(&self) -> CoreResult<()> {
        let mut directories = tokio::fs::read_dir(&self.root)
            .await
            .map_err(|_| unavailable())?;
        while let Some(dir) = directories.next_entry().await.map_err(|_| unavailable())? {
            if !dir.file_type().await.map_err(|_| unavailable())?.is_dir() {
                return Err(unavailable());
            }
            let id = dir.file_name().to_string_lossy().into_owned();
            uuid::Uuid::parse_str(&id).map_err(|_| unavailable())?;
            let registered = self
                .chunk_rows(&id)
                .await?
                .iter()
                .map(|row| row.try_get::<String>("", "file_name"))
                .collect::<Result<std::collections::HashSet<_>, _>>()
                .map_err(|_| unavailable())?;
            let mut files = tokio::fs::read_dir(dir.path())
                .await
                .map_err(|_| unavailable())?;
            while let Some(file) = files.next_entry().await.map_err(|_| unavailable())? {
                if !file.file_type().await.map_err(|_| unavailable())?.is_file() {
                    return Err(unavailable());
                }
                if registered.contains(&file.file_name().to_string_lossy().into_owned()) {
                    continue;
                }
                let age = file
                    .metadata()
                    .await
                    .map_err(|_| unavailable())?
                    .modified()
                    .map_err(|_| unavailable())?
                    .elapsed()
                    .unwrap_or_default();
                if age.as_secs() >= 300 {
                    let bytes = file.metadata().await.map_err(|_| unavailable())?.len();
                    tokio::fs::remove_file(file.path())
                        .await
                        .map_err(|_| unavailable())?;
                    self.disk_usage
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                            Some(used.saturating_sub(bytes))
                        })
                        .map_err(|_| unavailable())?;
                }
            }
        }
        Ok(())
    }
}
