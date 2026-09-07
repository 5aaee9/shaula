//! GitHub Auth Profile persistence: revisions with credential bytes,
//! promotion (staged activation) and exact-ref retrieval for the GitHub
//! Access Module.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use crate::entities::auth::{github_auth_profile_revisions, github_auth_profiles};
use crate::entities::fleet::fleet_auth_handoffs;
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
    /// head. Identity fields (kind, app/installation, principal, allowlist)
    /// are fixed per incarnation and validated by the caller.
    pub(crate) async fn auth_commit_revision(
        &self,
        tx: &DatabaseTransaction,
        insert: shaula_core::auth::AuthRevisionInsert,
        credential_bytes: &[u8],
        now: i64,
    ) -> StoreResult<()> {
        let key = &insert.key;
        let existing = github_auth_profiles::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?;
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

    /// Applies validation outcome: promote (staged activation) or reject
    /// while keeping the previous active revision untouched. A promotion
    /// ALSO re-triggers the auth handoff of every live fleet pinned to this
    /// profile (spec 0002 §299) in the SAME transaction, so a rotation can
    /// never leave fleets stranded on the pre-rotation credential.
    pub(crate) async fn auth_apply_validation(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
    ) -> StoreResult<()> {
        let tx = self.begin().await?;
        let result = self
            .auth_apply_validation_tx(&tx, key, revision, accepted, reason, now)
            .await;
        match result {
            Ok(()) => tx.commit().await.map_err(StoreError::from),
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    async fn auth_apply_validation_tx(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
    ) -> StoreResult<()> {
        // R10-07: BOTH reads happen ON THE TRANSACTION. Reading through a
        // separate connection would let a concurrent PUT advance the
        // desired head between read and commit, and the stale revision
        // write would then clobber the newer state.
        let candidate = github_auth_profile_revisions::Entity::find()
            .filter(github_auth_profile_revisions::Column::ProfileKey.eq(key))
            .filter(github_auth_profile_revisions::Column::Revision.eq(revision))
            .one(tx)
            .await?
            .ok_or_else(|| {
                StoreError::Corrupt(format!("auth revision {key}/{revision} missing"))
            })?;
        let mut candidate_updated: github_auth_profile_revisions::ActiveModel = candidate.into();
        candidate_updated.state = Set(if accepted {
            "Active".to_string()
        } else {
            "Rejected".to_string()
        });
        candidate_updated.reason = Set(reason.map(str::to_string));
        github_auth_profile_revisions::Entity::update(candidate_updated)
            .exec(tx)
            .await?;

        let profile = github_auth_profiles::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("auth profile {key} missing")))?;
        if profile.deletion_requested {
            return Err(StoreError::Conflict {
                resource: format!("{key} is retiring"),
            });
        }
        let desired_matches = profile.desired_revision == revision;
        if !desired_matches {
            return Err(StoreError::Conflict {
                resource: format!("{key}/{revision} is no longer desired"),
            });
        }
        let rejected_status = if profile.active_revision.is_some() {
            "Active"
        } else {
            "Rejected"
        };
        let mut updated: github_auth_profiles::ActiveModel = profile.into();
        if accepted {
            updated.active_revision = Set(Some(revision));
            updated.observed_revision = Set(Some(revision));
            updated.status = Set("Active".to_string());
        } else {
            updated.status = Set(rejected_status.to_string());
        }
        updated.updated_at = Set(now);
        github_auth_profiles::Entity::update(updated)
            .exec(tx)
            .await?;

        if accepted {
            self.retarget_fleets_on_rotation(tx, key, revision).await?;
        }
        use crate::entities::template::profile_changes;
        profile_changes::Entity::update_many()
            .col_expr(
                profile_changes::Column::State,
                sea_orm::sea_query::Expr::value(if accepted { "Converged" } else { "Rejected" }),
            )
            .col_expr(
                profile_changes::Column::Reason,
                sea_orm::sea_query::Expr::value(reason.map(str::to_string)),
            )
            .col_expr(
                profile_changes::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(profile_changes::Column::ResourceKind.eq("github_auth_profile"))
            .filter(profile_changes::Column::ProfileKey.eq(key))
            .filter(profile_changes::Column::Revision.eq(revision))
            .filter(profile_changes::Column::Kind.eq("Rotate"))
            .exec(tx)
            .await?;
        Ok(())
    }

    /// Points the handoff desired tuple of every fleet CURRENTLY depending on
    /// this auth profile (desired_profile_key == key in the handoff table)
    /// at the newly active revision. Selecting by CURRENT dependency — not
    /// by historical fleet revisions — means a fleet that already switched
    /// to a different profile is never dragged back; the update-only
    /// retarget never resurrects a removed handoff.
    async fn retarget_fleets_on_rotation(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        active_revision: i64,
    ) -> StoreResult<()> {
        let depending = fleet_auth_handoffs::Entity::find()
            .filter(fleet_auth_handoffs::Column::DesiredProfileKey.eq(key.to_string()))
            .all(tx)
            .await?;
        for handoff in depending {
            let fleet_key = handoff.fleet_key;
            // The no-op decision belongs to handoff_retarget_desired, which
            // compares the handoff's CURRENT desired tuple with the target.
            // The fleet's own revision counter and the auth profile's
            // revision counter are independent sequences and must never be
            // numerically compared here.
            self.handoff_retarget_desired(tx, &fleet_key, key, active_revision)
                .await?;
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
}

impl Store {}
