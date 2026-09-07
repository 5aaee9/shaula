use sea_orm::{ConnectionTrait, FromQueryResult, Value};
use shaula_core::state_backend::{
    LockInfo, StateAccess, StateClaim, StateDocument, StateError, StateResult, StateSnapshot,
};

use super::{sql, unavailable};

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
        let row = conn
            .query_one(sql(
                "SELECT s.generation_id, s.worker_epoch, s.worker_attempt, s.capability_hash,
                    s.revoked, s.sealed, s.create_started, s.revision, s.lineage, s.serial, s.lock_id
                 FROM generation_http_state s JOIN runner_generations g ON g.id = s.generation_id
                 WHERE s.generation_id = ?",
                vec![id.into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unauthorized)?;
        Self::from_query_result(&row, "").map_err(unavailable)
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

    pub fn identity_values(&self) -> Vec<Value> {
        vec![
            self.generation_id.clone().into(),
            self.worker_epoch.into(),
            self.worker_attempt.clone().into(),
        ]
    }

    pub async fn snapshot(
        &self,
        conn: &impl ConnectionTrait,
    ) -> StateResult<Option<StateSnapshot>> {
        let raw = conn
            .query_one(sql(
                "SELECT state_bytes FROM generation_http_state WHERE generation_id = ?",
                vec![self.generation_id.clone().into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?;
        let bytes: Option<Vec<u8>> = raw.try_get("", "state_bytes").map_err(unavailable)?;
        match bytes {
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

    pub async fn lock(&self, conn: &impl ConnectionTrait) -> StateResult<Option<LockInfo>> {
        let raw = conn
            .query_one(sql(
                "SELECT lock_info FROM generation_http_state WHERE generation_id = ?",
                vec![self.generation_id.clone().into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Unavailable)?;
        let bytes: Option<Vec<u8>> = raw.try_get("", "lock_info").map_err(unavailable)?;
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
