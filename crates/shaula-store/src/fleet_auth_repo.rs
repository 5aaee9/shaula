//! Fleet auth-handoff persistence, split to keep the host file within
//! the 400-line limit (AGENTS.md).

use sea_orm::ActiveValue::Set;
use sea_orm::{DatabaseTransaction, EntityTrait};

use crate::entities::fleet::fleet_auth_handoffs;
use crate::store::{Store, StoreResult};

impl Store {
    // ---- Auth handoff state ----

    pub(crate) async fn handoff_get(
        &self,
        key: &str,
    ) -> StoreResult<Option<fleet_auth_handoffs::Model>> {
        Ok(fleet_auth_handoffs::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn handoff_set_desired(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        profile_key: &str,
        revision: i64,
    ) -> StoreResult<()> {
        let existing = fleet_auth_handoffs::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?;
        match existing {
            Some(row) => {
                let mut updated: fleet_auth_handoffs::ActiveModel = row.into();
                updated.desired_profile_key = Set(profile_key.to_string());
                updated.desired_revision = Set(revision);
                updated.state = Set("Pending".to_string());
                updated.reason = Set(None);
                fleet_auth_handoffs::Entity::update(updated)
                    .exec(tx)
                    .await?;
            }
            None => {
                let row = fleet_auth_handoffs::ActiveModel {
                    fleet_key: Set(key.to_string()),
                    desired_profile_key: Set(profile_key.to_string()),
                    desired_revision: Set(revision),
                    observed_profile_key: Set(None),
                    observed_revision: Set(None),
                    state: Set("Pending".to_string()),
                    cleanup_only: Set(false),
                    attempts: Set(0),
                    next_retry_at: Set(None),
                    lease_owner: Set(None),
                    lease_expires_at: Set(None),
                    reason: Set(None),
                };
                fleet_auth_handoffs::Entity::insert(row).exec(tx).await?;
            }
        }
        Ok(())
    }

    /// Points an EXISTING handoff at a newly active auth revision after a
    /// rotation (spec 0002 section 299). Update-only: never resurrects a
    /// removed row, and clears the backoff because a Blocked reason
    /// referred to the old credential. CLEANUP-ONLY handoffs ARE
    /// retargeted: a decommissioning fleet waiting on a busy runner may
    /// need the fresh credential to finish cleanup - the cleanup-only
    /// flag gates session/acquire/Create effects, not the credential
    /// reference itself.
    pub(crate) async fn handoff_retarget_desired(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        profile_key: &str,
        revision: i64,
    ) -> StoreResult<()> {
        let Some(row) = fleet_auth_handoffs::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?
        else {
            return Ok(());
        };
        if row.desired_profile_key == profile_key && row.desired_revision == revision {
            return Ok(());
        }
        let mut updated: fleet_auth_handoffs::ActiveModel = row.into();
        updated.desired_profile_key = Set(profile_key.to_string());
        updated.desired_revision = Set(revision);
        updated.state = Set("Pending".to_string());
        updated.reason = Set(None);
        updated.next_retry_at = Set(None);
        fleet_auth_handoffs::Entity::update(updated)
            .exec(tx)
            .await?;
        Ok(())
    }

    pub(crate) async fn handoff_set_cleanup_only(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
    ) -> StoreResult<()> {
        // Update-only: a fleet without a handoff row simply has nothing to
        // mark; decommission must not invent one.
        let Some(row) = fleet_auth_handoffs::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?
        else {
            return Ok(());
        };
        let mut updated: fleet_auth_handoffs::ActiveModel = row.into();
        updated.cleanup_only = Set(true);
        fleet_auth_handoffs::Entity::update(updated)
            .exec(tx)
            .await?;
        Ok(())
    }

    /// Durably advances the observed tuple to the full desired tuple after
    /// successful classification.
    pub(crate) async fn handoff_acknowledge(
        &self,
        key: &str,
        profile_key: &str,
        revision: i64,
    ) -> StoreResult<()> {
        let row = fleet_auth_handoffs::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| crate::store::StoreError::Corrupt(format!("handoff {key} missing")))?;
        let mut updated: fleet_auth_handoffs::ActiveModel = row.into();
        updated.observed_profile_key = Set(Some(profile_key.to_string()));
        updated.observed_revision = Set(Some(revision));
        updated.state = Set("Observed".to_string());
        updated.reason = Set(None);
        fleet_auth_handoffs::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }

    pub(crate) async fn handoff_mark_blocked(
        &self,
        key: &str,
        reason: &str,
        next_retry_at: i64,
    ) -> StoreResult<()> {
        let row = fleet_auth_handoffs::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| crate::store::StoreError::Corrupt(format!("handoff {key} missing")))?;
        let attempts = row.attempts + 1;
        let mut updated: fleet_auth_handoffs::ActiveModel = row.into();
        updated.state = Set("Blocked".to_string());
        updated.reason = Set(Some(reason.to_string()));
        updated.next_retry_at = Set(Some(next_retry_at));
        updated.attempts = Set(attempts);
        fleet_auth_handoffs::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }
}
