use async_trait::async_trait;
use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};
use shaula_core::state_backend::{
    LockId, LockInfo, StateAccess, StateBackend, StateDocument, StateError, StateResult,
    StateSnapshot,
};

use super::{exactly_one, row::Row, states, unavailable, workers, SqliteStateBackend};

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
            let result = states::Entity::update_many()
                .col_expr(states::Column::LockId, Expr::value(info.id().expose()))
                .col_expr(states::Column::LockInfo, Expr::value(info.to_bytes()?))
                .filter(row.owned())
                .filter(states::Column::LockId.is_null())
                .filter(states::Column::Revoked.eq(false))
                .filter(states::Column::Sealed.eq(false))
                .exec(&tx)
                .await
                .map_err(unavailable)?;
            exactly_one(result.rows_affected)?;
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
            let result = states::Entity::update_many()
                .col_expr(states::Column::LockId, Expr::value(Option::<String>::None))
                .col_expr(
                    states::Column::LockInfo,
                    Expr::value(Option::<Vec<u8>>::None),
                )
                .filter(row.owned())
                .filter(states::Column::LockId.eq(id.expose()))
                .filter(states::Column::Revoked.eq(false))
                .filter(states::Column::Sealed.eq(false))
                .exec(&tx)
                .await
                .map_err(unavailable)?;
            exactly_one(result.rows_affected)?;
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
        // Lock, owner, serial/lineage and revision all belong to this SAME
        // writer transaction. Conditional UPDATE is a second, explicit fence.
        let result = states::Entity::update_many()
            .col_expr(
                states::Column::StateBytes,
                Expr::value(document.bytes().to_vec()),
            )
            .col_expr(states::Column::Lineage, Expr::value(document.lineage()))
            .col_expr(states::Column::Serial, Expr::value(document.serial()))
            .col_expr(states::Column::Revision, Expr::value(revision))
            .filter(row.owned())
            .filter(states::Column::Revision.eq(row.revision))
            .filter(states::Column::LockId.eq(id.expose()))
            .filter(states::Column::Revoked.eq(false))
            .filter(states::Column::Sealed.eq(false))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        exactly_one(result.rows_affected)?;
        if !document.managed_empty() {
            workers::Entity::update_many()
                .col_expr(workers::Column::ResourceStateSeen, Expr::value(true))
                .filter(workers::Column::GenerationId.eq(access.generation_id.to_string()))
                .exec(&tx)
                .await
                .map_err(unavailable)?;
        }
        tx.commit().await.map_err(unavailable)?;
        Ok(revision)
    }
}
