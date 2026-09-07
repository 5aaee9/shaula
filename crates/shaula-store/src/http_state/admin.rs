use sea_orm::ConnectionTrait;
use shaula_core::{
    lifecycle::GenerationState,
    registry::GenerationRecord,
    state_backend::{StateCapability, StateClaim, StateError, StateResult},
};
use uuid::Uuid;

use super::{exactly_one, row::Row, sql, unavailable, SqliteStateBackend};
use crate::Store;

impl SqliteStateBackend {
    /// Storage half of new-generation admission. The daemon must first resolve
    /// exact material/capacity admission under its Fleet effect gate. Inserting
    /// the Generation and fresh backend is atomic; an EXISTING Generation is
    /// always refused, including legacy local-state rows. No migration by 404.
    ///
    /// This grants state access only, never JIT or permission to spawn apply.
    pub async fn insert_generation(
        &self,
        record: GenerationRecord,
        worker_attempt: Uuid,
    ) -> StateResult<(StateClaim, StateCapability)> {
        let id = Uuid::parse_str(&record.id).map_err(|_| StateError::Invalid)?;
        if record.id != id.to_string()
            || record.state != GenerationState::CreatePending
            || record.github_runner_id.is_some()
        {
            return Err(StateError::Invalid);
        }
        let tx = self.writer(&record.id).await?;
        let exists = tx
            .query_one(sql(
                "SELECT id FROM runner_generations WHERE id = ?",
                vec![record.id.clone().into()],
            ))
            .await
            .map_err(unavailable)?;
        if exists.is_some() {
            return Err(StateError::Conflict);
        }
        let fleet = tx
            .query_one(sql(
                "SELECT desired_revision, deletion_marker, tombstone FROM fleets WHERE key = ?",
                vec![record.fleet_key.clone().into()],
            ))
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Conflict)?;
        let revision: i64 = fleet.try_get("", "desired_revision").map_err(unavailable)?;
        let deleting: bool = fleet.try_get("", "deletion_marker").map_err(unavailable)?;
        let tombstone: bool = fleet.try_get("", "tombstone").map_err(unavailable)?;
        if deleting || tombstone || revision != record.fleet_revision {
            return Err(StateError::Conflict);
        }
        let capability = StateCapability::issue();
        let claim = StateClaim {
            generation_id: id,
            worker_epoch: 1,
            worker_attempt,
        };
        Store::generation_insert_on(&tx, record)
            .await
            .map_err(|_| StateError::Unavailable)?;
        tx.execute(sql(
            "INSERT INTO generation_http_state
                (generation_id, worker_epoch, worker_attempt, capability_hash) VALUES (?, ?, ?, ?)",
            vec![
                id.to_string().into(),
                claim.worker_epoch.into(),
                worker_attempt.to_string().into(),
                capability.verifier().into(),
            ],
        ))
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        Ok((claim, capability))
    }

    /// Revoke network authority without changing state, lock, epoch or resource
    /// Occupancy. Revocation is NOT descendant fencing and permits no takeover.
    pub async fn revoke(&self, claim: &StateClaim) -> StateResult<()> {
        let tx = self.writer(&claim.generation_id.to_string()).await?;
        let row = Row::load(&tx, &claim.generation_id.to_string()).await?;
        row.check_claim(claim)?;
        let result = tx
            .execute(sql(
                "UPDATE generation_http_state SET revoked = 1
             WHERE generation_id = ? AND worker_epoch = ? AND worker_attempt = ?",
                row.identity_values(),
            ))
            .await
            .map_err(unavailable)?;
        exactly_one(result.rows_affected())?;
        tx.commit().await.map_err(unavailable)
    }

    /// Backend-side missing-state fence, NOT the Fleet Create-start handshake.
    /// The caller must have initialized authoritative empty state before the
    /// first possible Create, then retain its Fleet gate through spawn handover.
    /// This fact alone never authorizes an apply, repeated or otherwise.
    pub async fn note_create_starting(&self, claim: &StateClaim) -> StateResult<()> {
        let tx = self.writer(&claim.generation_id.to_string()).await?;
        let row = Row::load(&tx, &claim.generation_id.to_string()).await?;
        row.check_claim(claim)?;
        row.writable()?;
        if row.revoked || row.create_started {
            return Err(StateError::Conflict);
        }
        row.snapshot(&tx).await?.ok_or(StateError::Unavailable)?;
        let result = tx
            .execute(sql(
                "UPDATE generation_http_state SET create_started = 1
             WHERE generation_id = ? AND worker_epoch = ? AND worker_attempt = ?
                AND create_started = 0 AND sealed = 0 AND revoked = 0",
                row.identity_values(),
            ))
            .await
            .map_err(unavailable)?;
        exactly_one(result.rows_affected())?;
        tx.commit().await.map_err(unavailable)
    }

    /// Backend-only write seal. This neither releases Occupancy nor authorizes
    /// Workspace deletion. The future completion use case must combine this
    /// transaction with its trusted GitHub/Destroy receipt, not infer cleanup
    /// from empty state. No worker/control HTTP route exposes this primitive.
    pub async fn seal(&self, claim: &StateClaim, expected_revision: i64) -> StateResult<()> {
        let tx = self.writer(&claim.generation_id.to_string()).await?;
        let row = Row::load(&tx, &claim.generation_id.to_string()).await?;
        row.check_claim(claim)?;
        if row.revoked || !row.create_started || row.revision != expected_revision {
            return Err(StateError::Conflict);
        }
        let snapshot = row.snapshot(&tx).await?.ok_or(StateError::Unavailable)?;
        if !snapshot.document.managed_empty() || row.lock(&tx).await?.is_some() {
            return Err(StateError::Conflict);
        }
        let mut values = row.identity_values();
        values.push(expected_revision.into());
        let result = tx
            .execute(sql(
                "UPDATE generation_http_state SET sealed = 1
             WHERE generation_id = ? AND worker_epoch = ? AND worker_attempt = ?
                AND revision = ? AND lock_id IS NULL AND revoked = 0",
                values,
            ))
            .await
            .map_err(unavailable)?;
        exactly_one(result.rows_affected())?;
        tx.commit().await.map_err(unavailable)
    }
}
