//! Session installation, invalidation and demand snapshots.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, QueryFilter, Statement};
use shaula_core::registry::{PersistedSession, SessionInstall};

use crate::entities::lifecycle::{fleet_demand, fleet_sessions, job_observations, scale_set_state};
use crate::runtime_guards::session_active;
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    /// Atomically publishes the full listener handle, its initial demand,
    /// and retained exact authentication under the captured head and epoch.
    pub(crate) async fn session_install(
        &self,
        fleet_key: &str,
        install: &SessionInstall,
        now: i64,
    ) -> StoreResult<Option<i64>> {
        if install.handle.session_id.is_empty()
            || install.handle.message_queue_url.is_empty()
            || install.handle.message_queue_access_token.is_empty()
            || install.handle.initial_statistics.total_assigned_jobs < 0
        {
            return Err(StoreError::Corrupt("incomplete session handle".into()));
        }
        let tx = self.begin().await?;
        if !self
            .runtime_auth_guard_tx(&tx, fleet_key, &install.guard, &install.auth_context)
            .await?
        {
            return Ok(None);
        }
        let ownership = scale_set_state::Entity::find_by_id(fleet_key)
            .one(&tx)
            .await?;
        if !ownership
            .is_some_and(|o| o.scale_set_id == Some(install.scale_set_id) && o.state == "Adopted")
        {
            return Ok(None);
        }
        let previous = fleet_sessions::Entity::find_by_id(fleet_key)
            .one(&tx)
            .await?;
        if previous.as_ref().map(|s| s.epoch) != install.expected_epoch {
            return Ok(None);
        }
        let epoch = previous
            .as_ref()
            .map_or(0, |s| s.epoch)
            .checked_add(1)
            .ok_or_else(|| StoreError::Corrupt("session epoch exhausted".into()))?;
        if let Some(previous) = &previous {
            self.listener_session_end_tx(&tx, fleet_key, previous.epoch)
                .await?;
        }
        let row = fleet_sessions::ActiveModel {
            fleet_key: Set(fleet_key.to_string()),
            session_id: Set(install.handle.session_id.clone()),
            epoch: Set(epoch),
            scale_set_id: Set(install.scale_set_id),
            message_queue_url: Set(Some(install.handle.message_queue_url.clone())),
            queue_token: Set(Some(install.handle.message_queue_access_token.clone())),
            last_message_id: Set(0),
            created_at: Set(now),
        };
        fleet_sessions::Entity::insert(row)
            .on_conflict(
                sea_orm::sea_query::OnConflict::column(fleet_sessions::Column::FleetKey)
                    .update_columns([
                        fleet_sessions::Column::SessionId,
                        fleet_sessions::Column::Epoch,
                        fleet_sessions::Column::ScaleSetId,
                        fleet_sessions::Column::MessageQueueUrl,
                        fleet_sessions::Column::QueueToken,
                        fleet_sessions::Column::LastMessageId,
                        fleet_sessions::Column::CreatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&tx)
            .await?;
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO fleet_session_auth(fleet_key,profile_key,revision) VALUES(?,?,?)
             ON CONFLICT(fleet_key) DO UPDATE SET profile_key=excluded.profile_key, revision=excluded.revision",
            [fleet_key.into(), install.auth_context.profile_key.clone().into(), install.auth_context.revision.into()],
        )).await?;
        Self::demand_snapshot_tx(
            &tx,
            fleet_key,
            install.handle.initial_statistics.total_assigned_jobs,
            now,
        )
        .await?;
        self.listener_reconcile_old_tx(&tx, fleet_key, epoch, now)
            .await?;
        tx.commit().await?;
        Ok(Some(epoch))
    }

    pub(crate) async fn session_get(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Option<fleet_sessions::Model>> {
        Ok(fleet_sessions::Entity::find_by_id(fleet_key)
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn session_handle(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Option<PersistedSession>> {
        let tx = self.begin().await?;
        let Some(row) = fleet_sessions::Entity::find_by_id(fleet_key)
            .one(&tx)
            .await?
        else {
            return Ok(None);
        };
        if !session_active(&row) {
            return Ok(None);
        }
        let auth_context = self
            .session_auth_context_tx(&tx, fleet_key)
            .await?
            .ok_or_else(|| {
                StoreError::Corrupt("active session exact auth context missing".into())
            })?;
        let demand = fleet_demand::Entity::find_by_id(fleet_key)
            .one(&tx)
            .await?
            .map_or(0, |d| d.total_assigned_jobs);
        let session = PersistedSession {
            epoch: row.epoch,
            last_message_id: row.last_message_id,
            scale_set_id: row.scale_set_id,
            auth_context,
            handle: shaula_core::ports::SessionHandle {
                session_id: row.session_id,
                message_queue_url: row.message_queue_url.unwrap_or_default(),
                message_queue_access_token: row.queue_token.unwrap_or_default(),
                initial_statistics: shaula_core::ports::StatisticsSnapshot {
                    total_assigned_jobs: demand,
                    ..Default::default()
                },
            },
        };
        tx.commit().await?;
        Ok(Some(session))
    }

    /// Keep the epoch row after clearing to prevent an ABA session identity.
    pub(crate) async fn session_clear(
        &self,
        fleet_key: &str,
        expected_epoch: i64,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.begin().await?;
        let Some(row) = fleet_sessions::Entity::find_by_id(fleet_key)
            .one(&tx)
            .await?
        else {
            return Ok(false);
        };
        if row.epoch != expected_epoch {
            return Ok(false);
        }
        self.listener_session_end_tx(&tx, fleet_key, expected_epoch)
            .await?;
        let epoch = row
            .epoch
            .checked_add(1)
            .ok_or_else(|| StoreError::Corrupt("session epoch exhausted".into()))?;
        let mut updated: fleet_sessions::ActiveModel = row.into();
        updated.epoch = Set(epoch);
        updated.session_id = Set(String::new());
        updated.message_queue_url = Set(None);
        updated.queue_token = Set(None);
        updated.last_message_id = Set(0);
        updated.created_at = Set(now);
        fleet_sessions::Entity::update(updated).exec(&tx).await?;
        tx.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "DELETE FROM fleet_session_auth WHERE fleet_key=?",
            [fleet_key.into()],
        ))
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The fleet head guard tuple: `(desired_revision, deleting)`.
    pub(crate) async fn fleet_head_guard(
        &self,
        key: &str,
    ) -> StoreResult<Option<shaula_core::registry::FleetHeadGuard>> {
        use crate::entities::fleet::fleets;
        Ok(fleets::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?
            .map(|f| (f.desired_revision, f.deletion_marker || f.tombstone)))
    }

    // ---- Demand ----

    pub(crate) async fn demand_snapshot(
        &self,
        fleet_key: &str,
        total_assigned_jobs: i64,
        now: i64,
    ) -> StoreResult<()> {
        Self::demand_snapshot_tx(self.connection(), fleet_key, total_assigned_jobs, now).await
    }

    pub(crate) async fn demand_snapshot_tx<C: ConnectionTrait>(
        connection: &C,
        fleet_key: &str,
        total_assigned_jobs: i64,
        now: i64,
    ) -> StoreResult<()> {
        let row = fleet_demand::ActiveModel {
            fleet_key: Set(fleet_key.to_string()),
            total_assigned_jobs: Set(total_assigned_jobs),
            updated_at: Set(now),
        };
        fleet_demand::Entity::insert(row)
            .on_conflict(
                sea_orm::sea_query::OnConflict::column(fleet_demand::Column::FleetKey)
                    .update_columns([
                        fleet_demand::Column::TotalAssignedJobs,
                        fleet_demand::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(connection)
            .await?;
        Ok(())
    }

    pub(crate) async fn demand_get(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Option<fleet_demand::Model>> {
        Ok(fleet_demand::Entity::find_by_id(fleet_key.to_string())
            .one(self.connection())
            .await?)
    }

    // ---- Job observations (idempotent) ----

    pub async fn observation_insert(
        &self,
        insert: shaula_core::registry::JobObservationInsert,
    ) -> StoreResult<bool> {
        let now = insert.now;
        // Duplicate (fleet, message, kind, request) tuples are ignored so
        // redelivery stays idempotent.
        let existing = job_observations::Entity::find()
            .filter(job_observations::Column::FleetKey.eq(insert.fleet_key.as_str()))
            .filter(job_observations::Column::MessageId.eq(insert.message_id))
            .filter(job_observations::Column::ObservationKind.eq(insert.observation_kind.as_str()))
            .filter(job_observations::Column::RunnerRequestId.eq(insert.runner_request_id))
            .one(self.connection())
            .await?;
        if existing.is_some() {
            return Ok(false);
        }
        job_observations::Entity::insert(job_observations::ActiveModel {
            id: Default::default(),
            fleet_key: Set(insert.fleet_key.clone()),
            message_id: Set(insert.message_id),
            observation_kind: Set(insert.observation_kind.clone()),
            runner_request_id: Set(insert.runner_request_id),
            job_id: Set(insert.job_id.clone()),
            runner_name: Set(insert.runner_name.clone()),
            observed_at: Set(now),
        })
        .exec(self.connection())
        .await?;
        Ok(true)
    }
}
