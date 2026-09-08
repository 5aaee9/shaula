//! GitHub Auth Profile persistence: revisions with credential bytes,
//! staged-activation head management and exact-ref retrieval for the
//! GitHub Access Module. Promotion/gating lives in `auth_v2_repo.rs`.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use crate::entities::auth::{github_auth_profile_revisions, github_auth_profiles};
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    pub(crate) async fn auth_profile_get(
        &self,
        key: &str,
    ) -> StoreResult<Option<github_auth_profiles::Model>> {
        Ok(github_auth_profiles::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn auth_profiles_list(&self) -> StoreResult<Vec<github_auth_profiles::Model>> {
        Ok(github_auth_profiles::Entity::find()
            .all(self.connection())
            .await?)
    }

    /// Creates a Candidate credential revision and advances the desired
    /// head. Identity fields (kind, app/installation, principal, allowlist
    /// or v2 policy) are fixed per incarnation and validated by the caller.
    /// A v2 Candidate carries `policy_json` and an EMPTY legacy allowlist —
    /// an old binary reading the row fails closed instead of mistaking the
    /// first binding for the single installation.
    pub(crate) async fn auth_commit_revision(
        &self,
        tx: &DatabaseTransaction,
        insert: shaula_core::auth::AuthRevisionInsert,
        credential_bytes: &[u8],
        now: i64,
    ) -> StoreResult<()> {
        let key = &insert.key;
        // Acquire SQLite's writer with the revision INSERT before reading the
        // head. A deferred SELECT first permits two snapshots of r1; the loser
        // cannot upgrade its stale snapshot and returns SQLITE_BUSY (HTTP 500)
        // instead of the domain conflict (412). The unique revision constraint
        // serializes contenders before the read without a separate lock table.
        let row = github_auth_profile_revisions::ActiveModel {
            id: Default::default(),
            profile_key: Set(key.to_string()),
            revision: Set(insert.revision),
            kind: Set(insert.kind.clone()),
            app_id: Set(insert.app_id.clone()),
            installation_id: Set(insert.installation_id),
            pat_principal: Set(insert.pat_principal.clone()),
            allowlist_json: Set(insert.allowlist_json.clone()),
            credential_bytes: Set(credential_bytes.to_vec()),
            state: Set("Validating".to_string()),
            reason: Set(None),
            created_at: Set(now),
            schema_version: Set(insert.schema_version),
            policy_json: Set(insert.policy_json.clone()),
            validation_snapshot_json: Set(None),
        };
        github_auth_profile_revisions::Entity::insert(row)
            .exec(tx)
            .await
            .map_err(|e| {
                if e.to_string().contains("UNIQUE") {
                    StoreError::Conflict {
                        resource: "auth revision raced".into(),
                    }
                } else {
                    StoreError::from(e)
                }
            })?;

        let existing = github_auth_profiles::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?;
        match existing {
            None => {
                let profile = github_auth_profiles::ActiveModel {
                    key: Set(key.to_string()),
                    incarnation: Set(insert.incarnation.clone()),
                    desired_revision: Set(insert.revision),
                    active_revision: Set(None),
                    observed_revision: Set(None),
                    status: Set("Validating".to_string()),
                    deletion_requested: Set(false),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                github_auth_profiles::Entity::insert(profile)
                    .exec(tx)
                    .await?;
            }
            Some(current) => {
                if current.incarnation != insert.incarnation
                    || current.desired_revision.checked_add(1) != Some(insert.revision)
                    || current.deletion_requested
                {
                    return Err(StoreError::Conflict {
                        resource: key.to_string(),
                    });
                }
                let mut updated: github_auth_profiles::ActiveModel = current.into();
                updated.desired_revision = Set(insert.revision);
                updated.status = Set("Validating".to_string());
                updated.updated_at = Set(now);
                github_auth_profiles::Entity::update(updated)
                    .exec(tx)
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn auth_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> StoreResult<Option<github_auth_profile_revisions::Model>> {
        Ok(github_auth_profile_revisions::Entity::find()
            .filter(github_auth_profile_revisions::Column::ProfileKey.eq(key))
            .filter(github_auth_profile_revisions::Column::Revision.eq(revision))
            .one(self.connection())
            .await?)
    }

    /// Latest active revision model, for building the GitHub client from
    /// the exact accepted credential.
    pub(crate) async fn auth_revision_active(
        &self,
        key: &str,
    ) -> StoreResult<Option<github_auth_profile_revisions::Model>> {
        let profile = self
            .auth_profile_get(key)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("auth profile {key} missing")))?;
        let Some(active) = profile.active_revision else {
            return Ok(None);
        };
        self.auth_revision_get(key, active).await
    }

    /// Loads the revision ON THE TRANSACTION (the promotion transaction's
    /// serializable read of the Candidate).
    pub(crate) async fn auth_revision_get_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        revision: i64,
    ) -> StoreResult<Option<github_auth_profile_revisions::Model>> {
        Ok(github_auth_profile_revisions::Entity::find()
            .filter(github_auth_profile_revisions::Column::ProfileKey.eq(key))
            .filter(github_auth_profile_revisions::Column::Revision.eq(revision))
            .one(tx)
            .await?)
    }
}
