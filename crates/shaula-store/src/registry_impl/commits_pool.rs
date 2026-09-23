//! Shared TemplatePool commit implementations (spec 0037): revision
//! append with immutable member rows, and the reference-checked delete.

use shaula_core::error::CoreResult;
use shaula_core::registry::{MutationError, MutationFacts};

use super::lifecycle_support::fence_conflict;
use super::{core_err, SqliteControlPlane};

impl SqliteControlPlane {
    pub(crate) async fn commit_template_pool_mutation_impl(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        // Admission may have observed Active before DELETE reserved the writer.
        // Only exact pins already owned by this live pool may survive its fence.
        for member in &facts.template_pool {
            use sea_orm::{ConnectionTrait, DbBackend, Statement};
            let retired = tx.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
                "SELECT 1 FROM template_profiles t WHERE t.key=? AND t.deletion_requested=1
                 AND NOT EXISTS(SELECT 1 FROM template_pool_members m JOIN template_pools p
                    ON p.key=m.pool_key AND p.desired_revision=m.pool_revision
                    WHERE p.key=? AND p.tombstone=0 AND m.template_profile_key=t.key AND m.template_revision=?)",
                [member.template_profile_key.clone().into(), facts.resource_key.clone().into(), member.template_revision.into()],
            )).await.map_err(|e| core_err(e.into()))?;
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
        self.store
            .audit_append(
                tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: "template_pool".into(),
                    action: action.into(),
                    actor: facts.actor.clone(),
                    resource_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    outcome: "accepted".into(),
                    detail_json: None,
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .outbox_enqueue(
                tx,
                "template_pool",
                &facts.outbox_topic,
                &facts.outbox_payload,
                facts.now,
            )
            .await
            .map_err(core_err)?;
        if let Some((idem_key, request_hash, status, body)) = &facts.idempotency {
            self.store
                .idempotency_store(
                    tx,
                    shaula_core::registry::IdempotencyInsert {
                        id: format!("idem-{}", facts.change.id),
                        resource_kind: facts.resource_kind.to_string(),
                        resource_key: facts.resource_key.clone(),
                        idempotency_key: idem_key.clone(),
                        request_hash: request_hash.clone(),
                        response_status: *status,
                        response_body: Some(body.clone()),
                        now: facts.now,
                    },
                )
                .await
                .map_err(core_err)?;
        }
        Ok(())
    }
}
