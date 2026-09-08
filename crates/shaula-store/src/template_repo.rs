//! Template Profile persistence: revisions, activation, attestations,
//! artifact registry entries and profile changes.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use crate::entities::template::{
    profile_changes, template_artifacts, template_conformance_attestations,
    template_profile_revisions, template_profiles,
};
use crate::store::{Store, StoreError, StoreResult};

impl Store {
    pub(crate) async fn template_profile_get(
        &self,
        key: &str,
    ) -> StoreResult<Option<template_profiles::Model>> {
        Ok(template_profiles::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn template_profiles_list(
        &self,
    ) -> StoreResult<Vec<template_profiles::Model>> {
        Ok(template_profiles::Entity::find()
            .all(self.connection())
            .await?)
    }

    pub(crate) async fn template_revision_get(
        &self,
        key: &str,
        revision: i64,
    ) -> StoreResult<Option<template_profile_revisions::Model>> {
        Ok(template_profile_revisions::Entity::find()
            .filter(template_profile_revisions::Column::ProfileKey.eq(key))
            .filter(template_profile_revisions::Column::Revision.eq(revision))
            .one(self.connection())
            .await?)
    }

    /// Creates a new Candidate revision and advances the desired head.
    pub(crate) async fn template_commit_revision(
        &self,
        tx: &DatabaseTransaction,
        insert: shaula_core::template::TemplateRevisionInsert,
        now: i64,
    ) -> StoreResult<()> {
        let key = &insert.key;
        let existing = template_profiles::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?;
        let row = template_profile_revisions::ActiveModel {
            id: Default::default(),
            profile_key: Set(key.to_string()),
            revision: Set(insert.revision),
            artifact_digest: Set(insert.artifact_digest.clone()),
            engine_ref: Set(insert.engine_ref.clone()),
            platform: Set(None),
            bindings_contract: Set(None),
            manifest_json: Set(None),
            lock_digest: Set(None),
            bindings_json: Set(insert.bindings_json.clone()),
            bindings_digest: Set(insert.bindings_digest.clone()),
            fleet_input_policy_json: Set(insert.fleet_input_policy_json.clone()),
            state: Set("Validating".to_string()),
            reason: Set(None),
            created_at: Set(now),
        };
        template_profile_revisions::Entity::insert(row)
            .exec(tx)
            .await
            .map_err(|e| {
                if e.to_string().contains("UNIQUE") {
                    StoreError::Conflict {
                        resource: "template revision raced".into(),
                    }
                } else {
                    StoreError::from(e)
                }
            })?;

        match existing {
            None => {
                let profile = template_profiles::ActiveModel {
                    key: Set(key.to_string()),
                    incarnation: Set(insert.incarnation.clone()),
                    desired_revision: Set(insert.revision),
                    active_revision: Set(None),
                    observed_revision: Set(None),
                    active_attestation_id: Set(None),
                    status: Set("Validating".to_string()),
                    deletion_requested: Set(false),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                template_profiles::Entity::insert(profile).exec(tx).await?;
            }
            Some(current) => {
                if current.incarnation != insert.incarnation || current.deletion_requested {
                    return Err(StoreError::Conflict {
                        resource: key.to_string(),
                    });
                }
                // Mutation-fence CAS (R9-02): the caller must have read
                // this exact desired revision — a concurrent PUT that
                // advanced the head between admission and commit can never
                // be overwritten.
                if current.desired_revision.checked_add(1) != Some(insert.revision) {
                    return Err(StoreError::Conflict {
                        resource: format!(
                            "{key}:stale-desired-revision {} vs {}",
                            current.desired_revision, insert.revision
                        ),
                    });
                }
                let mut updated: template_profiles::ActiveModel = current.into();
                updated.desired_revision = Set(insert.revision);
                updated.status = Set("Validating".to_string());
                updated.updated_at = Set(now);
                template_profiles::Entity::update(updated).exec(tx).await?;
            }
        }
        Ok(())
    }

    /// Looks up an attestation by its STABLE composite identity inside
    /// a transaction — the in-commit replay/conflict authority for
    /// attestation PUT (R5-09).
    pub(crate) async fn template_attestation_get(
        &self,
        tx: &DatabaseTransaction,
        id: &str,
    ) -> StoreResult<Option<template_conformance_attestations::Model>> {
        Ok(
            template_conformance_attestations::Entity::find_by_id(id.to_string())
                .one(tx)
                .await?,
        )
    }

    /// Same lookup WITHOUT a transaction: the SERVICE-level replay
    /// pre-check reaches the immutable historical record before the
    /// current engine authority is consulted (R6-07). Crate-private
    /// (R7-07/C21): the SeaORM Model must not escape the store boundary —
    /// the daemon only ever sees the core `AttestationReplayRow` mapping.
    pub(crate) async fn template_attestation_get_by_id(
        &self,
        id: &str,
    ) -> StoreResult<Option<template_conformance_attestations::Model>> {
        Ok(
            template_conformance_attestations::Entity::find_by_id(id.to_string())
                .one(self.connection())
                .await?,
        )
    }

    pub(crate) async fn template_attestation_insert(
        &self,
        tx: &DatabaseTransaction,
        insert: shaula_core::template::AttestationInsert,
    ) -> StoreResult<()> {
        let row = template_conformance_attestations::ActiveModel {
            id: Set(insert.id.clone()),
            profile_key: Set(insert.key.clone()),
            revision: Set(insert.revision),
            subject_json: Set(insert.subject_json.clone()),
            subject_digest: Set(insert.subject_digest.clone()),
            result: Set(insert.result.clone()),
            evidence_digest: Set(insert.evidence_digest.clone()),
            suite_name: Set(insert.suite_name.clone()),
            suite_version: Set(insert.suite_version.clone()),
            completed_at: Set(insert.completed_at),
            subject_verified: Set(insert.subject_verified),
        };
        template_conformance_attestations::Entity::insert(row)
            .exec(tx)
            .await?;
        Ok(())
    }

    /// Digest-idempotent artifact registration; returns whether this call
    /// created the entry.
    pub async fn template_artifact_register(
        &self,
        digest: &str,
        size_bytes: i64,
        now: i64,
    ) -> StoreResult<bool> {
        if template_artifacts::Entity::find_by_id(digest.to_string())
            .one(self.connection())
            .await?
            .is_some()
        {
            return Ok(false);
        }
        let row = template_artifacts::ActiveModel {
            digest: Set(digest.to_string()),
            size_bytes: Set(size_bytes),
            state: Set("Orphan".to_string()),
            reference_count: Set(0),
            created_at: Set(now),
        };
        template_artifacts::Entity::insert(row)
            .exec(self.connection())
            .await?;
        Ok(true)
    }

    pub(crate) async fn profile_change_insert(
        &self,
        tx: &DatabaseTransaction,
        insert: shaula_core::registry::ProfileChangeInsert,
    ) -> StoreResult<()> {
        let now = insert.now;
        let row = profile_changes::ActiveModel {
            id: Set(insert.id.clone()),
            resource_kind: Set(insert.resource_kind.clone()),
            profile_key: Set(insert.profile_key.clone()),
            revision: Set(insert.revision),
            kind: Set(insert.kind.clone()),
            state: Set("Pending".to_string()),
            attempts: Set(0),
            next_retry_at: Set(None),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            reason: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        profile_changes::Entity::insert(row).exec(tx).await?;
        Ok(())
    }

    pub(crate) async fn profile_change_update(
        &self,
        id: &str,
        state: &str,
        reason: Option<&str>,
        next_retry_at: Option<i64>,
        now: i64,
    ) -> StoreResult<()> {
        let row = profile_changes::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("profile change {id} missing")))?;
        let mut updated: profile_changes::ActiveModel = row.into();
        updated.state = Set(state.to_string());
        updated.reason = Set(reason.map(str::to_string));
        updated.next_retry_at = Set(next_retry_at);
        updated.updated_at = Set(now);
        profile_changes::Entity::update(updated)
            .exec(self.connection())
            .await?;
        Ok(())
    }

    pub(crate) async fn profile_change_get(
        &self,
        id: &str,
    ) -> StoreResult<Option<profile_changes::Model>> {
        Ok(profile_changes::Entity::find_by_id(id.to_string())
            .one(self.connection())
            .await?)
    }
}

impl Store {
    /// Applies the structured credential validation outcome for an Auth
    /// Candidate: promote to Active (staged activation) or reject while
    /// keeping the prior active revision untouched. Delegates to the full
    /// v2-aware promotion in `auth_v2_repo`.
    pub(crate) async fn auth_scan_apply(
        &self,
        key: &str,
        revision: i64,
        accepted: bool,
        reason: Option<&str>,
        now: i64,
    ) -> StoreResult<()> {
        self.auth_apply_full(key, revision, accepted, reason, now, None)
            .await
            .map(|_| ())
    }
}
