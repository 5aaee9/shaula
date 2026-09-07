//! Fleet registry persistence: admission transactions, revisions, changes,
//! auth handoff state and shared audit/outbox/idempotency helpers reused by
//! the other repositories.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::auth::github_auth_profiles;
use crate::entities::fleet::{fleet_changes, fleet_revisions, fleets};
use crate::store::{Store, StoreResult};

impl Store {
    pub(crate) async fn fleet_get(&self, key: &str) -> StoreResult<Option<fleets::Model>> {
        Ok(fleets::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn fleet_list(&self) -> StoreResult<Vec<fleets::Model>> {
        Ok(fleets::Entity::find()
            .filter(fleets::Column::Tombstone.eq(false))
            .all(self.connection())
            .await?)
    }

    pub(crate) async fn fleet_revision_latest(
        &self,
        key: &str,
    ) -> StoreResult<Option<fleet_revisions::Model>> {
        Ok(fleet_revisions::Entity::find()
            .filter(fleet_revisions::Column::FleetKey.eq(key))
            .order_by_desc(fleet_revisions::Column::Revision)
            .one(self.connection())
            .await?)
    }

    /// Creates or updates the fleet desired head and appends an immutable
    /// revision inside the given transaction (normal mutation path). The
    /// mutation-fence CAS re-runs IN the transaction so a concurrent
    /// DELETE between admission and commit cannot be overwritten.
    pub(crate) async fn fleet_commit_revision(
        &self,
        tx: &DatabaseTransaction,
        insert: shaula_core::fleet::FleetRevisionInsert,
    ) -> StoreResult<()> {
        // R9-03: the auth snapshot is re-resolved INSIDE the transaction.
        // A concurrent auth promotion (activation is monotonic per
        // profile) must never be rolled back by a capacity PUT that read
        // a stale revision outside — the row records the CURRENT active
        // revision (spec 0002 §7).
        let auth_desired = if insert.auth_desired.0.is_empty() {
            (String::new(), 0)
        } else {
            let profile = github_auth_profiles::Entity::find_by_id(insert.auth_desired.0.clone())
                .one(tx)
                .await?
                .ok_or_else(|| crate::store::StoreError::Conflict {
                    resource: format!("{}:auth-profile-missing", insert.auth_desired.0),
                })?;
            let revision =
                profile
                    .active_revision
                    .ok_or_else(|| crate::store::StoreError::Conflict {
                        resource: format!("{}:auth-profile-not-active", insert.auth_desired.0),
                    })?;
            (insert.auth_desired.0.clone(), revision)
        };
        let key = &insert.key;
        let existing = fleets::Entity::find_by_id(key.to_string()).one(tx).await?;
        let (template_key, template_revision, artifact_digest, attestation_id) =
            match &insert.template {
                Some(t) => (
                    Some(t.0.clone()),
                    Some(t.1),
                    Some(t.2.clone()),
                    Some(t.3.clone()),
                ),
                None => (None, None, None, None),
            };
        let revision_row = fleet_revisions::ActiveModel {
            fleet_key: Set(key.to_string()),
            incarnation: Set(insert.incarnation.clone()),
            revision: Set(insert.revision),
            spec_json: Set(insert.spec_json.clone()),
            template_profile_key: Set(template_key),
            template_revision: Set(template_revision),
            template_artifact_digest: Set(artifact_digest),
            template_attestation_id: Set(attestation_id),
            auth_desired_profile_key: Set(auth_desired.0.clone()),
            auth_desired_revision: Set(auth_desired.1),
            inputs_digest: Set(insert.inputs_digest.clone()),
            actor: Set(Some(insert.actor.clone())),
            created_at: Set(insert.now),
            ..Default::default()
        };
        fleet_revisions::Entity::insert(revision_row)
            .exec(tx)
            .await?;

        match existing {
            None => {
                let row = fleets::ActiveModel {
                    key: Set(key.to_string()),
                    incarnation: Set(insert.incarnation.clone()),
                    desired_revision: Set(insert.revision),
                    observed_revision: Set(0),
                    mutation_fence: Set(1),
                    deletion_marker: Set(false),
                    phase: Set("Pending".to_string()),
                    tombstone: Set(false),
                    last_condition_reason: Set(None),
                    created_at: Set(insert.now),
                    updated_at: Set(insert.now),
                };
                fleets::Entity::insert(row).exec(tx).await?;
            }
            Some(current) => {
                if current.incarnation != insert.incarnation {
                    return Err(crate::store::StoreError::Conflict {
                        resource: key.to_string(),
                    });
                }
                // Mutation-fence CAS: the caller must have read this exact
                // desired revision, and the fleet must not be
                // decommissioning. The check re-runs inside the transaction
                // so a concurrent DELETE between admission and commit cannot
                // be overwritten.
                if current.deletion_marker {
                    return Err(crate::store::StoreError::Conflict {
                        resource: format!("{key}:decommissioning"),
                    });
                }
                if current.desired_revision + 1 != insert.revision {
                    return Err(crate::store::StoreError::Conflict {
                        resource: format!(
                            "{key}:stale-desired-revision {} vs {}",
                            current.desired_revision, insert.revision
                        ),
                    });
                }
                let next_fence = current.mutation_fence + 1;
                let mut updated: fleets::ActiveModel = current.into();
                updated.desired_revision = Set(insert.revision);
                updated.mutation_fence = Set(next_fence);
                updated.updated_at = Set(insert.now);
                fleets::Entity::update(updated).exec(tx).await?;
            }
        }
        Ok(())
    }

    /// Marks a decommission request: deletion marker plus a fence bump in
    /// one transaction.
    pub(crate) async fn fleet_mark_decommissioning(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        expected_incarnation: &str,
        new_revision: i64,
        now: i64,
    ) -> StoreResult<()> {
        // In-transaction CAS against the If-Match the client presented: a
        // concurrent PUT that advanced the desired head between admission
        // and commit must turn this DELETE into 412, never overwrite it
        // (spec 0002 section 5.1/8).
        let current = fleets::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?
            .ok_or_else(|| crate::store::StoreError::Conflict {
                resource: format!("{key}:missing"),
            })?;
        if current.incarnation != expected_incarnation
            || current.desired_revision + 1 != new_revision
        {
            return Err(crate::store::StoreError::Conflict {
                resource: format!(
                    "{key}:stale-decommission {} vs {}",
                    current.desired_revision, new_revision
                ),
            });
        }
        let next_fence = current.mutation_fence + 1;
        let mut updated: fleets::ActiveModel = current.into();
        updated.deletion_marker = Set(true);
        updated.desired_revision = Set(new_revision);
        updated.phase = Set("Decommissioning".to_string());
        updated.mutation_fence = Set(next_fence);
        updated.updated_at = Set(now);
        fleets::Entity::update(updated).exec(tx).await?;
        Ok(())
    }

    pub(crate) async fn fleet_set_observed(
        &self,
        key: &str,
        observed_revision: i64,
        phase: &str,
        reason: Option<&str>,
        now: i64,
    ) -> StoreResult<()> {
        let current = fleets::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| crate::store::StoreError::Corrupt(format!("fleet {key} missing")))?;
        let mut updated: fleets::ActiveModel = current.into();
        updated.observed_revision = Set(observed_revision);
        updated.phase = Set(phase.to_string());
        updated.last_condition_reason = Set(reason.map(str::to_string));
        updated.updated_at = Set(now);
        fleets::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }

    pub(crate) async fn fleet_set_tombstone(&self, key: &str, now: i64) -> StoreResult<()> {
        let current = fleets::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| crate::store::StoreError::Corrupt(format!("fleet {key} missing")))?;
        let mut updated: fleets::ActiveModel = current.into();
        updated.tombstone = Set(true);
        updated.phase = Set("Decommissioned".to_string());
        updated.updated_at = Set(now);
        fleets::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }

    // ---- Fleet changes ----

    pub(crate) async fn change_insert(
        &self,
        tx: &DatabaseTransaction,
        id: &str,
        key: &str,
        revision: i64,
        kind: &str,
        now: i64,
    ) -> StoreResult<()> {
        let row = fleet_changes::ActiveModel {
            id: Set(id.to_string()),
            fleet_key: Set(key.to_string()),
            revision: Set(revision),
            kind: Set(kind.to_string()),
            state: Set("Pending".to_string()),
            attempts: Set(0),
            next_retry_at: Set(None),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            reason: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        fleet_changes::Entity::insert(row).exec(tx).await?;
        Ok(())
    }

    pub(crate) async fn change_update(
        &self,
        id: &str,
        state: &str,
        reason: Option<&str>,
        next_retry_at: Option<i64>,
        now: i64,
    ) -> StoreResult<()> {
        let row = fleet_changes::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| crate::store::StoreError::Corrupt(format!("change {id} missing")))?;
        let mut updated: fleet_changes::ActiveModel = row.into();
        updated.state = Set(state.to_string());
        updated.reason = Set(reason.map(str::to_string));
        updated.next_retry_at = Set(next_retry_at);
        updated.updated_at = Set(now);
        fleet_changes::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }

    pub(crate) async fn change_get(&self, id: &str) -> StoreResult<Option<fleet_changes::Model>> {
        Ok(fleet_changes::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?)
    }
}
