//! Shared TemplatePool persistence (spec 0037): pool heads, immutable
//! revisions and resolved member rows, plus the reference-checked
//! tombstone path.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::fleet::{fleet_revisions, fleets};
use crate::entities::template_pool::{
    template_pool_members, template_pool_revisions, template_pools,
};
use crate::store::{Store, StoreResult};

/// Insert shape for one pool revision commit; members are the pool's own
/// resolved rows for this revision (spec 0037 §3).
pub(crate) struct TemplatePoolRevisionInsert {
    pub key: String,
    pub incarnation: String,
    pub revision: i64,
    pub spec_json: String,
    pub failure_policy: String,
    pub members: Vec<shaula_core::template_pool::ResolvedTemplatePoolMember>,
    pub actor: String,
    pub now: i64,
}

impl Store {
    pub(crate) async fn template_pool_get(
        &self,
        key: &str,
    ) -> StoreResult<Option<template_pools::Model>> {
        Ok(template_pools::Entity::find_by_id(key.to_string())
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn template_pool_list(&self) -> StoreResult<Vec<template_pools::Model>> {
        Ok(template_pools::Entity::find()
            .filter(template_pools::Column::Tombstone.eq(false))
            .all(self.connection())
            .await?)
    }

    pub(crate) async fn template_pool_revision_latest(
        &self,
        key: &str,
    ) -> StoreResult<Option<template_pool_revisions::Model>> {
        Ok(template_pool_revisions::Entity::find()
            .filter(template_pool_revisions::Column::PoolKey.eq(key))
            .order_by_desc(template_pool_revisions::Column::Revision)
            .one(self.connection())
            .await?)
    }

    pub(crate) async fn template_pool_members(
        &self,
        pool_key: &str,
        revision: i64,
    ) -> StoreResult<Vec<template_pool_members::Model>> {
        Ok(template_pool_members::Entity::find()
            .filter(template_pool_members::Column::PoolKey.eq(pool_key))
            .filter(template_pool_members::Column::PoolRevision.eq(revision))
            .all(self.connection())
            .await?)
    }

    /// Appends one pool revision and its immutable member rows, advancing
    /// the desired head with an in-transaction CAS on the incarnation and
    /// expected revision (mirrors `fleet_commit_revision` minus the fence:
    /// a pool owns no runners, so its commits race only other pool PUTs).
    pub(crate) async fn template_pool_commit_revision(
        &self,
        tx: &DatabaseTransaction,
        insert: TemplatePoolRevisionInsert,
    ) -> StoreResult<()> {
        let key = &insert.key;
        template_pool_revisions::Entity::insert(template_pool_revisions::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            pool_key: Set(key.clone()),
            revision: Set(insert.revision),
            spec_json: Set(insert.spec_json.clone()),
            failure_policy: Set(insert.failure_policy.clone()),
            actor: Set(Some(insert.actor.clone())),
            created_at: Set(insert.now),
        })
        .exec(tx)
        .await?;
        for member in &insert.members {
            template_pool_members::Entity::insert(template_pool_members::ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                pool_key: Set(key.clone()),
                pool_revision: Set(insert.revision),
                member_key: Set(member.key.clone()),
                template_profile_key: Set(member.template_profile_key.clone()),
                template_revision: Set(member.template_revision),
                template_artifact_digest: Set(member.template_artifact_digest.clone()),
                template_attestation_id: Set(member.template_attestation_id.clone()),
                template_inputs_json: Set(serde_json::to_string(&member.template_inputs)
                    .map_err(|e| crate::store::StoreError::Corrupt(e.to_string()))?),
                inputs_digest: Set(member.inputs_digest.clone()),
                weight: Set(i64::from(member.weight)),
                max_runners: Set(member.max_runners),
            })
            .exec(tx)
            .await?;
        }
        match template_pools::Entity::find_by_id(key.to_string())
            .one(tx)
            .await?
        {
            None => {
                template_pools::Entity::insert(template_pools::ActiveModel {
                    key: Set(key.clone()),
                    incarnation: Set(insert.incarnation.clone()),
                    desired_revision: Set(insert.revision),
                    observed_revision: Set(0),
                    phase: Set("Pending".to_string()),
                    deletion_marker: Set(false),
                    tombstone: Set(false),
                    created_at: Set(insert.now),
                    updated_at: Set(insert.now),
                })
                .exec(tx)
                .await?;
            }
            Some(current) => {
                if current.incarnation != insert.incarnation
                    || current.deletion_marker
                    || current.tombstone
                    || current.desired_revision + 1 != insert.revision
                {
                    return Err(crate::store::StoreError::Conflict {
                        resource: key.clone(),
                    });
                }
                let mut updated: template_pools::ActiveModel = current.into();
                updated.desired_revision = Set(insert.revision);
                updated.updated_at = Set(insert.now);
                template_pools::Entity::update(updated).exec(tx).await?;
            }
        }
        Ok(())
    }

    /// True while any LIVE fleet's LATEST revision still routes through
    /// this pool (spec 0037 §7): only the latest revision of a live fleet
    /// keeps routing future generations; older revisions are historical.
    pub(crate) async fn template_pool_referenced(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
    ) -> StoreResult<bool> {
        let live = fleets::Entity::find()
            .filter(fleets::Column::Tombstone.eq(false))
            .filter(fleets::Column::DeletionMarker.eq(false))
            .all(tx)
            .await?;
        for fleet in live {
            if let Some(latest) = fleet_revisions::Entity::find()
                .filter(fleet_revisions::Column::FleetKey.eq(&fleet.key))
                .order_by_desc(fleet_revisions::Column::Revision)
                .one(tx)
                .await?
            {
                if latest.template_pool_ref.as_deref() == Some(key) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Tombstones a pool. A pool owns no runners, so deletion is a
    /// reference-checked tombstone rather than a drain (spec 0037 §7).
    pub(crate) async fn template_pool_tombstone(
        &self,
        tx: &DatabaseTransaction,
        key: &str,
        expected_incarnation: &str,
        new_revision: i64,
        now: i64,
    ) -> StoreResult<()> {
        let current = template_pools::Entity::find_by_id(key.to_string())
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
                    "{key}:stale-delete {} vs {}",
                    current.desired_revision, new_revision
                ),
            });
        }
        let mut updated: template_pools::ActiveModel = current.into();
        updated.deletion_marker = Set(true);
        updated.tombstone = Set(true);
        updated.desired_revision = Set(new_revision);
        updated.phase = Set("Deleted".to_string());
        updated.updated_at = Set(now);
        template_pools::Entity::update(updated).exec(tx).await?;
        Ok(())
    }
}
