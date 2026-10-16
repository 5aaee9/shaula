use sea_orm::{
    sea_query::Expr, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::{
    lifecycle::GenerationState,
    registry::GenerationRecord,
    state_backend::{StateCapability, StateClaim, StateError, StateResult},
};
use uuid::Uuid;

use super::{exactly_one, row::Row, states, unavailable, SqliteStateBackend};
use crate::entities::{fleet::fleets, lifecycle::runner_generations as generations};
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
        let exists = generations::Entity::find_by_id(&record.id)
            .select_only()
            .column(generations::Column::Id)
            .into_tuple::<String>()
            .one(&tx)
            .await
            .map_err(unavailable)?;
        if exists.is_some() {
            return Err(StateError::Conflict);
        }
        let (revision, deleting, tombstone) = fleets::Entity::find_by_id(&record.fleet_key)
            .select_only()
            .columns([
                fleets::Column::DesiredRevision,
                fleets::Column::DeletionMarker,
                fleets::Column::Tombstone,
            ])
            .into_tuple::<(i64, bool, bool)>()
            .one(&tx)
            .await
            .map_err(unavailable)?
            .ok_or(StateError::Conflict)?;
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
        states::Entity::insert(states::ActiveModel {
            generation_id: Set(id.to_string()),
            worker_epoch: Set(claim.worker_epoch),
            worker_attempt: Set(worker_attempt.to_string()),
            capability_hash: Set(capability.verifier()),
            ..Default::default()
        })
        .exec_without_returning(&tx)
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
        let result = states::Entity::update_many()
            .col_expr(states::Column::Revoked, Expr::value(true))
            .filter(row.owned())
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        exactly_one(result.rows_affected)?;
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
        let result = states::Entity::update_many()
            .col_expr(states::Column::CreateStarted, Expr::value(true))
            .filter(row.owned())
            .filter(states::Column::CreateStarted.eq(false))
            .filter(states::Column::Sealed.eq(false))
            .filter(states::Column::Revoked.eq(false))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        exactly_one(result.rows_affected)?;
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
        let result = states::Entity::update_many()
            .col_expr(states::Column::Sealed, Expr::value(true))
            .filter(row.owned())
            .filter(states::Column::Revision.eq(expected_revision))
            .filter(states::Column::LockId.is_null())
            .filter(states::Column::Revoked.eq(false))
            .exec(&tx)
            .await
            .map_err(unavailable)?;
        exactly_one(result.rows_affected)?;
        tx.commit().await.map_err(unavailable)
    }
}
