//! Fleet deletion advances the fence without writing a synthetic specification.
use super::{core_err, lifecycle_support::fence_conflict, SqliteControlPlane};
use crate::entities::fleet::fleet_revisions;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use shaula_core::{
    error::CoreResult,
    registry::{MutationError, MutationFacts},
};

impl SqliteControlPlane {
    pub(crate) async fn commit_decommission_impl(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        // The decommission path EXPECTS the deletion marker to be set;
        // fleet_mark_decommissioning also advances the desired head and
        // bumps the fence, so no pseudo-revision is written. An in-tx CAS
        // conflict is a DOMAIN rejection (412/410 via fence_conflict), not
        // an infrastructure error — never surface it as a 500.
        if let Err(e) = self
            .store
            .fleet_mark_decommissioning(
                &tx,
                &facts.resource_key,
                &facts.incarnation,
                facts.revision,
                facts.now,
            )
            .await
        {
            if matches!(e, crate::store::StoreError::Conflict { .. }) {
                let _ = tx.rollback().await;
                return Ok(Err(fence_conflict(&facts, &e)));
            }
            return Err(core_err(e));
        }
        let forgejo = fleet_revisions::Entity::find()
            .filter(fleet_revisions::Column::FleetKey.eq(&facts.resource_key))
            .order_by_desc(fleet_revisions::Column::Revision)
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
            .and_then(|row| {
                serde_json::from_str::<shaula_core::fleet::FleetSpec>(&row.spec_json).ok()
            })
            .is_some_and(|spec| spec.kind == shaula_core::fleet::FleetProviderKind::Forgejo);
        if !forgejo {
            if let Some((profile_key, revision)) = &facts.auth_desired {
                self.store
                    .handoff_set_desired(&tx, &facts.resource_key, profile_key, *revision)
                    .await
                    .map_err(core_err)?;
            }
            self.store
                .fleet_auth_context_refresh_fence_tx(&tx, &facts.resource_key, facts.now)
                .await
                .map_err(core_err)?;
            self.store
                .handoff_set_cleanup_only(&tx, &facts.resource_key)
                .await
                .map_err(core_err)?;
        }
        self.store
            .change_insert(
                &tx,
                &facts.change.id,
                &facts.resource_key,
                facts.revision,
                &facts.change.kind,
                facts.now,
            )
            .await
            .map_err(core_err)?;
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    authentication: facts.authentication.clone(),
                    resource_kind: facts.resource_kind.to_string(),
                    action: "put".into(),
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
                &tx,
                facts.resource_kind,
                &facts.outbox_topic,
                &facts.outbox_payload,
                facts.now,
            )
            .await
            .map_err(core_err)?;
        if let Some((idem_key, request_hash, status, body)) = &facts.idempotency {
            self.store
                .idempotency_store(
                    &tx,
                    shaula_core::registry::IdempotencyInsert {
                        operation: facts.idempotency_operation.into(),
                        principal: facts.actor.clone(),
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
        tx.commit().await.map_err(|e| core_err(e.into()))?;
        Ok(Ok(()))
    }
}
