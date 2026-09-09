//! Session epochs, demand snapshots and idempotent job observations,
//! split from `lifecycle_repo.rs` to keep every file within the
//! 400-line limit (AGENTS.md).

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::entities::lifecycle::{fleet_demand, fleet_sessions, job_observations};
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    /// Installs a session with the next monotonic epoch; returns the new
    /// epoch. Used under the per-fleet session-effect gate.
    pub(crate) async fn session_install(
        &self,
        fleet_key: &str,
        session_id: &str,
        scale_set_id: i64,
        now: i64,
    ) -> StoreResult<i64> {
        let tx = self.begin().await?;
        let handoff = crate::entities::fleet::fleet_auth_handoffs::Entity::find_by_id(fleet_key)
            .one(&tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt("session auth handoff missing".into()))?;
        let (key, revision) = handoff
            .observed_profile_key
            .as_deref()
            .zip(handoff.observed_revision)
            .ok_or_else(|| StoreError::Corrupt("session observed auth reference missing".into()))?;
        let auth = self
            .auth_revision_get_tx(&tx, key, revision)
            .await?
            .ok_or_else(|| StoreError::Corrupt("session auth revision missing".into()))?;
        if auth.schema_version != 2 || auth.kind != "github_app" {
            return Err(StoreError::PolicyDenied {
                reason: "UnsupportedAuthFormat",
            });
        }
        if self
            .auth_execution_context_tx(&tx, fleet_key, key, revision)
            .await?
            .is_none()
        {
            return Err(StoreError::Corrupt(
                "session requires observed exact auth context".into(),
            ));
        }
        let previous = fleet_sessions::Entity::find_by_id(fleet_key.to_string())
            .one(&tx)
            .await?;
        let epoch = match previous {
            None => {
                let row = fleet_sessions::ActiveModel {
                    fleet_key: Set(fleet_key.to_string()),
                    session_id: Set(session_id.to_string()),
                    epoch: Set(1),
                    scale_set_id: Set(scale_set_id),
                    message_queue_url: Set(None),
                    queue_token: Set(None),
                    last_message_id: Set(0),
                    created_at: Set(now),
                };
                fleet_sessions::Entity::insert(row).exec(&tx).await?;
                1
            }
            Some(row) => {
                let epoch = row.epoch + 1;
                let mut updated: fleet_sessions::ActiveModel = row.into();
                updated.session_id = Set(session_id.to_string());
                updated.epoch = Set(epoch);
                updated.scale_set_id = Set(scale_set_id);
                updated.message_queue_url = Set(None);
                updated.queue_token = Set(None);
                updated.last_message_id = Set(0);
                fleet_sessions::Entity::update(updated).exec(&tx).await?;
                epoch
            }
        };
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        tx.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "DELETE FROM fleet_session_auth WHERE fleet_key=?",
            [fleet_key.into()],
        ))
        .await?;
        tx.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT OR REPLACE INTO fleet_session_auth(fleet_key,profile_key,revision)
             SELECT fleet_key,observed_profile_key,observed_revision FROM fleet_auth_handoffs
             WHERE fleet_key=? AND observed_profile_key IS NOT NULL AND observed_revision IS NOT NULL",
            [fleet_key.into()])).await?;
        tx.commit().await?;
        Ok(epoch)
    }

    pub(crate) async fn session_get(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Option<fleet_sessions::Model>> {
        Ok(fleet_sessions::Entity::find_by_id(fleet_key.to_string())
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn session_set_queue(
        &self,
        fleet_key: &str,
        url: &str,
        token: &str,
    ) -> StoreResult<()> {
        let row = fleet_sessions::Entity::find_by_id(fleet_key.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("session {fleet_key} missing")))?;
        let mut updated: fleet_sessions::ActiveModel = row.into();
        updated.message_queue_url = Set(Some(url.to_string()));
        updated.queue_token = Set(Some(token.to_string()));
        fleet_sessions::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
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
            .exec(self.connection())
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
