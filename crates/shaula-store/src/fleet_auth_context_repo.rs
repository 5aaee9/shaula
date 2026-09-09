//! Fleet exact Resolved Auth Context persistence (spec 0011 §4.2/§5.2).
//! The desired side is written inside the fleet mutation transaction from
//! the active revision's frozen bindings; the observed side advances only
//! through a CAS that compares the full desired ref AND refuses to
//! overwrite a conflicting durable identity pin.

use sea_orm::ActiveValue::Set;
use sea_orm::{DatabaseTransaction, EntityTrait};

use crate::entities::auth::fleet_auth_contexts;
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    /// Retiring a Fleet advances its fence without changing auth intent.
    /// Keep any pending cleanup proof acknowledgeable under that new fence.
    pub(crate) async fn fleet_auth_context_refresh_fence_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet_key: &str,
        now: i64,
    ) -> StoreResult<()> {
        use crate::entities::fleet::fleets;
        let Some(context) = fleet_auth_contexts::Entity::find_by_id(fleet_key.to_string())
            .one(tx)
            .await?
        else {
            return Ok(());
        };
        let fleet = fleets::Entity::find_by_id(fleet_key.to_string())
            .one(tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt("auth context has no Fleet".into()))?;
        let mut updated: fleet_auth_contexts::ActiveModel = context.into();
        updated.desired_fence = Set(Some(fleet.mutation_fence));
        updated.updated_at = Set(now);
        fleet_auth_contexts::Entity::update(updated)
            .exec(tx)
            .await?;
        Ok(())
    }

    /// Writes a required exact context and captures the current mutation fence.
    /// Observed authority is retained until acknowledgement succeeds.
    pub(crate) async fn fleet_auth_context_set_desired_tx(
        &self,
        tx: &DatabaseTransaction,
        fleet_key: &str,
        profile_key: &str,
        revision: i64,
        context_json: &str,
        now: i64,
    ) -> StoreResult<()> {
        use crate::entities::fleet::fleets;
        let fence = fleets::Entity::find_by_id(fleet_key.to_string())
            .one(tx)
            .await?
            .map(|f| f.mutation_fence);
        let existing = fleet_auth_contexts::Entity::find_by_id(fleet_key.to_string())
            .one(tx)
            .await?;
        match existing {
            Some(row) => {
                let desired_same = row.desired_profile_key.as_deref() == Some(profile_key)
                    && row.desired_revision == Some(revision)
                    && row.desired_context_json.as_deref() == Some(context_json);
                if desired_same {
                    // Same intent, NEW fence: refresh the captured fence so
                    // later acknowledgements stay valid (F7).
                    let mut fence_only: fleet_auth_contexts::ActiveModel = row.into();
                    fence_only.desired_fence = Set(fence);
                    fence_only.updated_at = Set(now);
                    fleet_auth_contexts::Entity::update(fence_only)
                        .exec(tx)
                        .await?;
                    return Ok(());
                }
                let mut updated: fleet_auth_contexts::ActiveModel = row.into();
                updated.desired_profile_key = Set(Some(profile_key.to_string()));
                updated.desired_revision = Set(Some(revision));
                updated.desired_fence = Set(fence);
                updated.desired_context_json = Set(Some(context_json.to_string()));
                updated.state = Set("Pending".to_string());
                updated.reason = Set(None);
                updated.next_retry_at = Set(None);
                updated.updated_at = Set(now);
                fleet_auth_contexts::Entity::update(updated)
                    .exec(tx)
                    .await?;
            }
            None => {
                let row = fleet_auth_contexts::ActiveModel {
                    fleet_key: Set(fleet_key.to_string()),
                    desired_profile_key: Set(Some(profile_key.to_string())),
                    desired_revision: Set(Some(revision)),
                    desired_fence: Set(fence),
                    desired_context_json: Set(Some(context_json.to_string())),
                    observed_profile_key: Set(None),
                    observed_revision: Set(None),
                    observed_context_json: Set(None),
                    state: Set("Pending".to_string()),
                    reason: Set(None),
                    attempts: Set(0),
                    next_retry_at: Set(None),
                    updated_at: Set(now),
                };
                fleet_auth_contexts::Entity::insert(row).exec(tx).await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn fleet_auth_context_get(
        &self,
        fleet_key: &str,
    ) -> StoreResult<Option<crate::entities::auth::fleet_auth_contexts::Model>> {
        Ok(
            fleet_auth_contexts::Entity::find_by_id(fleet_key.to_string())
                .one(self.connection())
                .await?,
        )
    }

    /// Marks the context resolution blocked (identity drift, unresolved
    /// binding) with a sanitized reason and bounded retry deadline.
    pub(crate) async fn fleet_auth_context_block(
        &self,
        fleet_key: &str,
        reason: &str,
        retry_at: i64,
        now: i64,
    ) -> StoreResult<()> {
        let Some(row) = self.fleet_auth_context_get(fleet_key).await? else {
            return Err(StoreError::Corrupt(format!("context {fleet_key} missing")));
        };
        let attempts = row.attempts + 1;
        let mut updated: fleet_auth_contexts::ActiveModel = row.into();
        updated.state = Set("Blocked".to_string());
        updated.reason = Set(Some(reason.to_string()));
        updated.next_retry_at = Set(Some(retry_at));
        updated.attempts = Set(attempts);
        updated.updated_at = Set(now);
        fleet_auth_contexts::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }
}
