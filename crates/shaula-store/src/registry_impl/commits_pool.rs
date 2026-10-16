//! Shared TemplatePool commit implementations (spec 0037): revision
//! append with immutable member rows, and the reference-checked delete.

use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use shaula_core::error::CoreResult;
use shaula_core::registry::{MutationError, MutationFacts};

use super::lifecycle_support::fence_conflict;
use super::{core_err, SqliteControlPlane};
use crate::entities::template::template_profiles as profiles;
use crate::entities::template_pool::{template_pool_members as members, template_pools as pools};
use crate::query_ext::{col, probe, same};

impl SqliteControlPlane {
    pub(crate) async fn commit_template_pool_mutation_impl(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        // Admission may have observed Active before DELETE reserved the writer.
        // Only exact pins already owned by this live pool may survive its fence.
        for member in &facts.template_pool {
            let pinned = probe(members::Entity)
                .inner_join(
                    pools::Entity,
                    same(pools::Column::Key, members::Column::PoolKey).and(same(
                        pools::Column::DesiredRevision,
                        members::Column::PoolRevision,
                    )),
                )
                .and_where(col(pools::Column::Key).eq(facts.resource_key.as_str()))
                .and_where(col(pools::Column::Tombstone).eq(false))
                .and_where(same(
                    members::Column::TemplateProfileKey,
                    profiles::Column::Key,
                ))
                .and_where(col(members::Column::TemplateRevision).eq(member.template_revision))
                .to_owned();
            let retired = profiles::Entity::find()
                .select_only()
                .expr(Expr::val(1))
                .filter(profiles::Column::Key.eq(member.template_profile_key.as_str()))
                .filter(profiles::Column::DeletionRequested.eq(true))
                .filter(Expr::exists(pinned).not())
                .into_tuple::<i32>()
                .one(&tx)
                .await
                .map_err(|e| core_err(e.into()))?;
            if retired.is_some() {
                return Ok(Err(MutationError::RetirementBlocked {
                    reason: "profile is retiring".into(),
                }));
            }
        }
        // The failure policy rides on the admitted spec JSON; the commit
        // persists it denormalized on the revision row for cheap reads.
        let failure_policy =
            serde_json::from_str::<shaula_core::template_pool::TemplatePoolSpec>(&facts.spec_json)
                .map(|spec| spec.failure_policy)
                .map_err(|error| {
                    core_err(crate::store::StoreError::Corrupt(format!(
                        "template pool revision {} has invalid spec: {error}",
                        facts.resource_key
                    )))
                })?;
        let failure_policy = match failure_policy {
            shaula_core::template_pool::PoolFailurePolicy::Backpressure => "backpressure",
            shaula_core::template_pool::PoolFailurePolicy::Redistribute => "redistribute",
        };
        if let Err(e) = self
            .store
            .template_pool_commit_revision(
                &tx,
                crate::pool_repo::TemplatePoolRevisionInsert {
                    key: facts.resource_key.clone(),
                    incarnation: facts.incarnation.clone(),
                    revision: facts.revision,
                    spec_json: facts.spec_json.clone(),
                    failure_policy: failure_policy.to_string(),
                    members: facts.template_pool.clone(),
                    actor: facts.actor.clone(),
                    now: facts.now,
                },
            )
            .await
        {
            // A lost head race maps to the HTTP precondition contract.
            return Ok(Err(fence_conflict(&facts, &e)));
        }
        self.pool_common_facts(&tx, &facts, "put").await?;
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }

    pub(crate) async fn commit_template_pool_delete_impl(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        // Reference check and tombstone share ONE transaction: a fleet
        // PUT that starts referencing the pool after this check fails
        // its own pool-exists admission against the tombstoned head.
        if self
            .store
            .template_pool_referenced(&tx, &facts.resource_key)
            .await
            .map_err(core_err)?
        {
            let _ = tx.rollback().await;
            return Ok(Err(MutationError::RetirementBlocked {
                reason: "template pool is referenced by a live fleet".into(),
            }));
        }
        if let Err(e) = self
            .store
            .template_pool_tombstone(
                &tx,
                &facts.resource_key,
                &facts.incarnation,
                facts.revision,
                facts.now,
            )
            .await
        {
            return Ok(Err(fence_conflict(&facts, &e)));
        }
        self.pool_common_facts(&tx, &facts, "delete").await?;
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }

    /// Change/audit/outbox/idempotency facts shared by pool mutations.
    async fn pool_common_facts(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        facts: &MutationFacts,
        action: &str,
    ) -> CoreResult<()> {
        self.store
            .profile_change_insert(
                tx,
                shaula_core::registry::ProfileChangeInsert {
                    id: facts.change.id.clone(),
                    resource_kind: "template_pool".into(),
                    profile_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    kind: facts.change.kind.clone(),
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.commit_tail(tx, facts, "template_pool", action, None)
            .await?;
        Ok(())
    }
}
