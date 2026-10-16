use super::{invalid, unavailable, OperationLogArchive};
use crate::entities::operation_logs::operation_log_chunks as log_chunks;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use sha2::{Digest, Sha256};
use shaula_core::error::CoreResult;
use std::collections::{HashMap, HashSet};
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

    pub(super) async fn chunk_rows(&self, id: &str) -> CoreResult<Vec<log_chunks::Model>> {
        log_chunks::Entity::find()
            .filter(log_chunks::Column::InvocationId.eq(id))
            .order_by_asc(log_chunks::Column::Sequence)
            .all(self.store.connection())
            .await
            .map_err(|_| unavailable())
    }

    pub(super) async fn chunk_text(&self, id: &str, row: &log_chunks::Model) -> CoreResult<String> {
        let sequence = row.sequence;
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
        if hex::encode(Sha256::digest(&bytes)) != row.digest {
            return Err(unavailable());
        }
        String::from_utf8(bytes).map_err(|_| unavailable())
    }

    pub(super) async fn remove_chunk(&self, id: &str, row: &log_chunks::Model) -> CoreResult<()> {
        let sequence = row.sequence;
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
        log_chunks::Entity::delete_many()
            .filter(log_chunks::Column::InvocationId.eq(id))
            .filter(log_chunks::Column::Sequence.eq(sequence))
            .exec(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        Ok(())
    }

    // One blocking walk: per-entry tokio::fs hops cost seconds at ~30k files
    // and pushed maintenance past its 5s budget on every tick.
    pub(super) async fn disk_bytes(&self) -> CoreResult<u64> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || scan_bytes(&root))
            .await
            .map_err(|_| unavailable())?
    }

    pub(super) async fn reap_orphans(&self) -> CoreResult<()> {
        // Callers hold the writer lock, so this snapshot cannot miss a publish.
        let rows = log_chunks::Entity::find()
            .select_only()
            .columns([
                log_chunks::Column::InvocationId,
                log_chunks::Column::FileName,
            ])
            .into_tuple::<(String, String)>()
            .all(self.store.connection())
            .await
            .map_err(|_| unavailable())?;
        let mut registered: HashMap<String, HashSet<String>> = HashMap::new();
        for (id, name) in rows {
            registered.entry(id).or_default().insert(name);
        }
        let root = self.root.clone();
        let (removed, outcome) = tokio::task::spawn_blocking(move || {
            let mut removed = 0u64;
            let outcome = reap_orphan_files(&root, &registered, &mut removed);
            (removed, outcome)
        })
        .await
        .map_err(|_| unavailable())?;
        self.disk_usage
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                Some(used.saturating_sub(removed))
            })
            .map_err(|_| unavailable())?;
        outcome
    }
}

fn scan_bytes(root: &Path) -> CoreResult<u64> {
    let mut used = 0u64;
    for dir in std::fs::read_dir(root).map_err(|_| unavailable())? {
        let dir = dir.map_err(|_| unavailable())?;
        if !dir.file_type().map_err(|_| unavailable())?.is_dir()
            || !dir.metadata().map_err(|_| unavailable())?.is_dir()
        {
            return Err(unavailable());
        }
        for file in std::fs::read_dir(dir.path()).map_err(|_| unavailable())? {
            let file = file.map_err(|_| unavailable())?;
            if !file.file_type().map_err(|_| unavailable())?.is_file() {
                return Err(unavailable());
            }
            used = used.saturating_add(file.metadata().map_err(|_| unavailable())?.len());
        }
    }
    Ok(used)
}

// `removed` accumulates even on a later error so the caller can still
// release reservations for files that are already gone.
fn reap_orphan_files(
    root: &Path,
    registered: &HashMap<String, HashSet<String>>,
    removed: &mut u64,
) -> CoreResult<()> {
    let empty = HashSet::new();
    for dir in std::fs::read_dir(root).map_err(|_| unavailable())? {
        let dir = dir.map_err(|_| unavailable())?;
        if !dir.file_type().map_err(|_| unavailable())?.is_dir() {
            return Err(unavailable());
        }
        let id = dir.file_name().to_string_lossy().into_owned();
        uuid::Uuid::parse_str(&id).map_err(|_| unavailable())?;
        let names = registered.get(&id).unwrap_or(&empty);
        for file in std::fs::read_dir(dir.path()).map_err(|_| unavailable())? {
            let file = file.map_err(|_| unavailable())?;
            if !file.file_type().map_err(|_| unavailable())?.is_file() {
                return Err(unavailable());
            }
            if names.contains(file.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            let meta = file.metadata().map_err(|_| unavailable())?;
            let age = meta
                .modified()
                .map_err(|_| unavailable())?
                .elapsed()
                .unwrap_or_default();
            if age.as_secs() >= 300 {
                std::fs::remove_file(file.path()).map_err(|_| unavailable())?;
                *removed = removed.saturating_add(meta.len());
            }
        }
    }
    Ok(())
}
