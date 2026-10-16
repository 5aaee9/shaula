use sea_orm::{
    sea_query::SimpleExpr, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QuerySelect,
};
use shaula_core::state_backend::{
    LockInfo, StateAccess, StateClaim, StateDocument, StateError, StateResult, StateSnapshot,
};

use super::{states, unavailable};
use crate::entities::lifecycle::runner_generations as generations;
use crate::query_ext::{same, JoinOn};

// Deliberately no Debug. Authentication reads metadata, NOT raw state/lock info.
#[derive(FromQueryResult)]
pub(super) struct Row {
    pub generation_id: String,
    pub worker_epoch: i64,
    pub worker_attempt: String,
    capability_hash: Vec<u8>,
    pub revoked: bool,
    pub sealed: bool,
    pub create_started: bool,
    pub revision: i64,
    lineage: Option<String>,
    serial: Option<i64>,
    lock_id: Option<String>,
}

impl Row {
    pub async fn load(conn: &impl ConnectionTrait, id: &str) -> StateResult<Self> {
        states::Entity::find_by_id(id)
            .select_only()
            .columns([
                states::Column::GenerationId,
                states::Column::WorkerEpoch,
                states::Column::WorkerAttempt,
                states::Column::CapabilityHash,
                states::Column::Revoked,
                states::Column::Sealed,
                states::Column::CreateStarted,
                states::Column::Revision,
                states::Column::Lineage,
                states::Column::Serial,
                states::Column::LockId,
            ])
            .inner_join_on(
                generations::Entity,
                same(generations::Column::Id, states::Column::GenerationId),
            )
            .into_model::<Self>()
            .one(conn)
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unauthorized)
    }

    pub fn authorize(&self, access: &StateAccess) -> StateResult<()> {
        if self.revoked || !access.capability.matches(&self.capability_hash) {
            return Err(StateError::Unauthorized);
        }
        Ok(())
    }

    pub fn check_claim(&self, claim: &StateClaim) -> StateResult<()> {
        if self.generation_id != claim.generation_id.to_string()
            || self.worker_epoch != claim.worker_epoch
            || self.worker_attempt != claim.worker_attempt.to_string()
        {
            return Err(StateError::Conflict);
        }
        Ok(())
    }

    pub fn writable(&self) -> StateResult<()> {
        if self.sealed {
            return Err(StateError::Sealed);
        }
        Ok(())
    }

    /// The exact owner this row was loaded with: Generation, epoch and attempt.
    pub fn owned(&self) -> SimpleExpr {
        states::Column::GenerationId
            .eq(&self.generation_id)
            .and(states::Column::WorkerEpoch.eq(self.worker_epoch))
            .and(states::Column::WorkerAttempt.eq(&self.worker_attempt))
    }

    async fn blob(
        &self,
        conn: &impl ConnectionTrait,
        column: states::Column,
    ) -> StateResult<Option<Vec<u8>>> {
        states::Entity::find_by_id(&self.generation_id)
            .select_only()
            .column(column)
            .into_tuple::<Option<Vec<u8>>>()
            .one(conn)
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)
    }

    pub async fn snapshot(
        &self,
        conn: &impl ConnectionTrait,
    ) -> StateResult<Option<StateSnapshot>> {
        match self.blob(conn, states::Column::StateBytes).await? {
            None if self.revision == 0
                && self.lineage.is_none()
                && self.serial.is_none()
                && !self.create_started
                && !self.sealed =>
            {
                Ok(None)
            }
            Some(bytes) if self.revision > 0 => {
                let document = StateDocument::parse(bytes).map_err(|_| StateError::Unavailable)?;
                if self.lineage.as_deref() != Some(document.lineage())
                    || self.serial != Some(document.serial())
                {
                    return Err(StateError::Unavailable);
                }
                Ok(Some(StateSnapshot {
                    revision: self.revision,
                    document,
                }))
            }
            _ => Err(StateError::Unavailable),
        }
    }

    /// The raw lock record, kept verbatim as fencing evidence.
    pub async fn lock_bytes(&self, conn: &impl ConnectionTrait) -> StateResult<Option<Vec<u8>>> {
        self.blob(conn, states::Column::LockInfo).await
    }

    pub async fn lock(&self, conn: &impl ConnectionTrait) -> StateResult<Option<LockInfo>> {
        let bytes = self.lock_bytes(conn).await?;
        match (self.lock_id.as_deref(), bytes) {
            (None, None) => Ok(None),
            (Some(id), Some(bytes)) => {
                let info = LockInfo::parse(&bytes).map_err(|_| StateError::Unavailable)?;
                if info.id().expose() != id {
                    return Err(StateError::Unavailable);
                }
                Ok(Some(info))
            }
            _ => Err(StateError::Unavailable),
        }
    }
}
