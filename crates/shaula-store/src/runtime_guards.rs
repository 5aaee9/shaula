//! Durable authority checks shared by listeners and runtime observations.

use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, EntityTrait, Statement};
use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::registry::FleetRuntimeGuard;

use crate::entities::{auth::fleet_auth_contexts, fleet::fleet_auth_handoffs};
use crate::entities::{fleet::fleets, lifecycle::fleet_sessions};
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    pub(crate) async fn session_close_authorize(
        &self,
        key: &str,
        guard: &FleetRuntimeGuard,
    ) -> StoreResult<bool> {
        Ok(fleets::Entity::find_by_id(key)
            .one(self.connection())
            .await?
            .is_some_and(|f| {
                f.incarnation == guard.incarnation
                    && f.desired_revision == guard.desired_revision
                    && f.mutation_fence == guard.mutation_fence
                    && !f.tombstone
            }))
    }

    pub(crate) async fn fleet_runtime_guard_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        guard: &FleetRuntimeGuard,
    ) -> StoreResult<bool> {
        Ok(fleets::Entity::find_by_id(key)
            .one(tx)
            .await?
            .is_some_and(|f| {
                f.incarnation == guard.incarnation
                    && f.desired_revision == guard.desired_revision
                    && f.mutation_fence == guard.mutation_fence
                    && !f.deletion_marker
                    && !f.tombstone
            }))
    }

    pub(crate) async fn runtime_auth_guard_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        guard: &FleetRuntimeGuard,
        expected: &ResolvedAuthContext,
    ) -> StoreResult<bool> {
        if !self.fleet_runtime_guard_tx(tx, key, guard).await? || !expected.has_complete_identity()
        {
            return Ok(false);
        }
        let Some(handoff) = fleet_auth_handoffs::Entity::find_by_id(key).one(tx).await? else {
            return Ok(false);
        };
        if handoff.state != "Observed"
            || handoff.cleanup_only
            || handoff.desired_profile_key != expected.profile_key
            || handoff.desired_revision != expected.revision
            || handoff.observed_profile_key.as_deref() != Some(expected.profile_key.as_str())
            || handoff.observed_revision != Some(expected.revision)
        {
            return Ok(false);
        }
        let Some(auth) = self
            .auth_revision_get_tx(tx, &expected.profile_key, expected.revision)
            .await?
        else {
            return Ok(false);
        };
        if auth.schema_version != 2 || auth.kind != "github_app" {
            return Err(StoreError::PolicyDenied {
                reason: "UnsupportedAuthFormat",
            });
        }
        let Some(row) = fleet_auth_contexts::Entity::find_by_id(key).one(tx).await? else {
            return Ok(false);
        };
        if row.state != "Observed"
            || row.desired_profile_key.as_deref() != Some(expected.profile_key.as_str())
            || row.desired_revision != Some(expected.revision)
            || row.desired_fence != Some(guard.mutation_fence)
            || row.observed_profile_key.as_deref() != Some(expected.profile_key.as_str())
            || row.observed_revision != Some(expected.revision)
        {
            return Ok(false);
        }
        let Some(desired) = decode_context(row.desired_context_json.as_deref())? else {
            return Ok(false);
        };
        let observed = decode_context(row.observed_context_json.as_deref())?;
        let retained = self
            .auth_execution_context_tx(tx, key, &expected.profile_key, expected.revision)
            .await?;
        Ok(expected.completes(&desired)
            && observed.as_ref() == Some(expected)
            && retained.as_ref() == Some(expected))
    }

    pub(crate) async fn session_guard_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        guard: &FleetRuntimeGuard,
        context: &ResolvedAuthContext,
        epoch: i64,
    ) -> StoreResult<bool> {
        if !self.runtime_auth_guard_tx(tx, key, guard, context).await? {
            return Ok(false);
        }
        let Some(session) = fleet_sessions::Entity::find_by_id(key).one(tx).await? else {
            return Ok(false);
        };
        if session.epoch != epoch || !session_active(&session) {
            return Ok(false);
        }
        let ownership = crate::entities::lifecycle::scale_set_state::Entity::find_by_id(key)
            .one(tx)
            .await?;
        if !ownership
            .is_some_and(|o| o.scale_set_id == Some(session.scale_set_id) && o.state == "Adopted")
        {
            return Ok(false);
        }
        let retained = self.session_auth_context_tx(tx, key).await?;
        Ok(retained.as_ref() == Some(context))
    }

    pub(crate) async fn session_auth_context_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
    ) -> StoreResult<Option<ResolvedAuthContext>> {
        let Some(row) = tx
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT profile_key, revision FROM fleet_session_auth WHERE fleet_key=?",
                [key.into()],
            ))
            .await?
        else {
            return Ok(None);
        };
        self.auth_execution_context_tx(
            tx,
            key,
            &row.try_get::<String>("", "profile_key")?,
            row.try_get("", "revision")?,
        )
        .await
    }
}

pub(crate) fn session_active(session: &fleet_sessions::Model) -> bool {
    !session.session_id.is_empty()
        && session
            .message_queue_url
            .as_ref()
            .is_some_and(|v| !v.is_empty())
        && session.queue_token.as_ref().is_some_and(|v| !v.is_empty())
}

fn decode_context(json: Option<&str>) -> StoreResult<Option<ResolvedAuthContext>> {
    json.map(|value| {
        serde_json::from_str(value)
            .map_err(|_| StoreError::Corrupt("session auth context unreadable".into()))
    })
    .transpose()
}
