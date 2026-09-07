use async_trait::async_trait;
use sea_orm::ConnectionTrait;
use shaula_core::state_backend::{
    LockId, LockInfo, StateAccess, StateBackend, StateDocument, StateError, StateResult,
    StateSnapshot,
};

use super::{exactly_one, row::Row, sql, unavailable, SqliteStateBackend};

#[async_trait]
impl StateBackend for SqliteStateBackend {
    async fn authenticate(&self, access: &StateAccess) -> StateResult<()> {
        Row::load(self.store.connection(), &access.generation_id.to_string())
            .await?
            .authorize(access)
    }

    async fn read(&self, access: &StateAccess) -> StateResult<Option<StateSnapshot>> {
        let tx = self
            .store
            .begin()
            .await
            .map_err(|_| StateError::Unavailable)?;
        let row = Row::load(&tx, &access.generation_id.to_string()).await?;
        row.authorize(access)?;
        let snapshot = row.snapshot(&tx).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(snapshot)
    }

    async fn lock(&self, access: &StateAccess, info: LockInfo) -> StateResult<()> {
        let tx = self.writer(&access.generation_id.to_string()).await?;
        let row = Row::load(&tx, &access.generation_id.to_string()).await?;
        row.authorize(access)?;
        row.writable()?;
        // Refuse effects on a lost/corrupt authoritative state. Only a truly
        // fresh Generation may initialize it under its first Terraform lock.
        row.snapshot(&tx).await?;
        if let Some(current) = row.lock(&tx).await? {
            if current != info {
                return Err(StateError::Locked(Box::new(current)));
            }
        } else {
            let mut values = vec![info.id().expose().into(), info.to_bytes()?.into()];
            values.extend(row.identity_values());
            let result = tx
                .execute(sql(
                    "UPDATE generation_http_state SET lock_id = ?, lock_info = ?
                 WHERE generation_id = ? AND worker_epoch = ? AND worker_attempt = ?
                    AND lock_id IS NULL AND revoked = 0 AND sealed = 0",
                    values,
                ))
                .await
                .map_err(unavailable)?;
            exactly_one(result.rows_affected())?;
        }
        tx.commit().await.map_err(unavailable)
    }

    async fn unlock(&self, access: &StateAccess, id: &LockId) -> StateResult<()> {
        let tx = self.writer(&access.generation_id.to_string()).await?;
        let row = Row::load(&tx, &access.generation_id.to_string()).await?;
        row.authorize(access)?;
        row.writable()?;
        if let Some(current) = row.lock(&tx).await? {
            if current.id() != id {
                return Err(StateError::Locked(Box::new(current)));
            }
            let mut values = row.identity_values();
            values.push(id.expose().into());
            let result = tx
                .execute(sql(
                    "UPDATE generation_http_state SET lock_id = NULL, lock_info = NULL
                 WHERE generation_id = ? AND worker_epoch = ? AND worker_attempt = ?
                    AND lock_id = ? AND revoked = 0 AND sealed = 0",
                    values,
                ))
                .await
                .map_err(unavailable)?;
            exactly_one(result.rows_affected())?;
        }
        // A lost UNLOCK response is harmless, but an intervening new lock must
        // have passed the exact-ID branch above. Never unlock by Who or Path.
        tx.commit().await.map_err(unavailable)
    }

    async fn write(
        &self,
        access: &StateAccess,
        id: &LockId,
        document: StateDocument,
    ) -> StateResult<i64> {
        let tx = self.writer(&access.generation_id.to_string()).await?;
        let row = Row::load(&tx, &access.generation_id.to_string()).await?;
        row.authorize(access)?;
        row.writable()?;
        let lock = row.lock(&tx).await?.ok_or(StateError::Conflict)?;
        if lock.id() != id {
            return Err(StateError::Locked(Box::new(lock)));
        }
        if let Some(previous) = row.snapshot(&tx).await? {
            if document.lineage() != previous.document.lineage()
                || document.serial() < previous.document.serial()
            {
                return Err(StateError::Conflict);
            }
            if document.serial() == previous.document.serial() {
                if document.bytes() != previous.document.bytes() {
                    return Err(StateError::Conflict);
                }
                tx.commit().await.map_err(unavailable)?;
                return Ok(row.revision);
            }
        }
        let revision = row.revision.checked_add(1).ok_or(StateError::Conflict)?;
        let mut values = vec![
            document.bytes().to_vec().into(),
            document.lineage().into(),
            document.serial().into(),
            revision.into(),
        ];
        values.extend(row.identity_values());
        values.extend([row.revision.into(), id.expose().into()]);
        // Lock, owner, serial/lineage and revision all belong to this SAME
        // writer transaction. Conditional UPDATE is a second, explicit fence.
        let result = tx.execute(sql(
            "UPDATE generation_http_state SET state_bytes = ?, lineage = ?, serial = ?, revision = ?
             WHERE generation_id = ? AND worker_epoch = ? AND worker_attempt = ?
                AND revision = ? AND lock_id = ? AND revoked = 0 AND sealed = 0",
            values,
        )).await.map_err(unavailable)?;
        exactly_one(result.rows_affected())?;
        tx.commit().await.map_err(unavailable)?;
        Ok(revision)
    }
}
