//! Fleet-specific commit implementations, split to keep files within
//! 400 lines (AGENTS.md).

use shaula_core::error::CoreResult;
use shaula_core::registry::{MutationError, MutationFacts};

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

use crate::entities::fleet::fleet_revisions;
use crate::entities::lifecycle::{runner_generations, runner_operations};

use super::lifecycle_support::fence_conflict;
use super::{core_err, SqliteControlPlane};

/// Resource occupancy counted INSIDE the commit transaction (R9-04):
/// every generation whose Destroy has not completed PLUS every in-flight
/// operation (an operation row can outlive its generation's state
/// transition). Any live resource blocks a template/auth replacement.
async fn fleet_replacement_occupancy_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    fleet_key: &str,
) -> crate::store::StoreResult<i64> {
    let generations = runner_generations::Entity::find()
        .filter(runner_generations::Column::FleetKey.eq(fleet_key))
        .all(tx)
        .await?;
    let mut occupancy = generations
        .iter()
        .filter(|g| g.state != "Destroyed")
        .count() as i64;
    let ids: Vec<String> = generations.iter().map(|g| g.id.clone()).collect();
    if !ids.is_empty() {
        occupancy += runner_operations::Entity::find()
            .filter(runner_operations::Column::GenerationId.is_in(ids))
            .filter(runner_operations::Column::State.is_in([
                "Pending",
                "Starting",
                "ApplyStarting",
                "BootstrapStarting",
                "Running",
                "Blocked",
            ]))
            .count(tx)
            .await? as i64;
    }
    Ok(occupancy)
}

impl SqliteControlPlane {
    pub(crate) async fn commit_fleet_mutation_impl(
        &self,
        facts: MutationFacts,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        // R10-08: the HEAD CAS write happens FIRST so this transaction
        // HOLDS the SQLite write lock before the occupancy verdict is
        // read — a deferred-transaction read before any write would let
        // a concurrent Create insert generations between the check and
        // the commit. The check is also re-run INSIDE the tx (R9-04):
        // any insertion after our write lock is impossible; anything
        // committed before it is counted.
        if let Err(e) = self
            .store
            .fleet_commit_revision(
                &tx,
                shaula_core::fleet::FleetRevisionInsert {
                    key: facts.resource_key.clone(),
                    incarnation: facts.incarnation.clone(),
                    revision: facts.revision,
                    spec_json: facts.spec_json.clone(),
                    template: facts.template.clone(),
                    auth_desired: facts.auth_desired.clone().unwrap_or_default(),
                    inputs_digest: facts.inputs_digest.clone(),
                    actor: facts.actor.clone(),
                    now: facts.now,
                },
            )
            .await
        {
            // Lost fence race: map the store conflict onto the HTTP status
            // contract (412/410) instead of an internal error.
            return Ok(Err(fence_conflict(&facts, &e)));
        }
        {
            let previous = fleet_revisions::Entity::find()
                .filter(fleet_revisions::Column::FleetKey.eq(&facts.resource_key))
                .filter(fleet_revisions::Column::Revision.lt(facts.revision))
                .order_by_desc(fleet_revisions::Column::Revision)
                .one(&tx)
                .await
                .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
            let new_template_key = facts.template.as_ref().map(|t| t.0.clone());
            if !super::retirement::allows_references(&tx, &facts, previous.as_ref())
                .await
                .map_err(core_err)?
            {
                tx.rollback().await.map_err(|e| core_err(e.into()))?;
                return Ok(Err(MutationError::RetirementBlocked {
                    reason: "profile is retiring".into(),
                }));
            }
            let new_template_revision = facts.template.as_ref().map(|t| t.1);
            let template_changed = previous.as_ref().is_some_and(|p| {
                p.template_profile_key != new_template_key
                    || p.template_revision != new_template_revision
            });
            let new_auth_key = facts
                .auth_desired
                .as_ref()
                .map(|(k, _)| k.clone())
                .unwrap_or_default();
            let auth_changed = previous
                .as_ref()
                .is_some_and(|p| p.auth_desired_profile_key.clone() != new_auth_key);
            if template_changed || auth_changed {
                let occupancy = fleet_replacement_occupancy_in_tx(&tx, &facts.resource_key)
                    .await
                    .map_err(core_err)?;
                if occupancy > 0 {
                    // R10-08: rollback discards the CAS write too — the
                    // replacement never lands over live resources.
                    let _ = tx.rollback().await;
                    return Ok(Err(MutationError::RetirementBlocked {
                        reason: "replacement requires zero resource occupancy".into(),
                    }));
                }
            }
        }
        if let Some((profile_key, _admission_revision)) = &facts.auth_desired {
            // R9-03: the handoff retarget also uses the IN-TX resolved
            // active revision — the revision row already carries it.
            let resolved = fleet_revisions::Entity::find()
                .filter(fleet_revisions::Column::FleetKey.eq(&facts.resource_key))
                .order_by_desc(fleet_revisions::Column::Revision)
                .one(&tx)
                .await
                .map_err(|e| core_err(crate::store::StoreError::from(e)))?
                .map(|r| r.auth_desired_revision)
                .unwrap_or(*_admission_revision);
            // Forgejo has no handoff or installation context. Its exact
            // credential reference lives on the immutable Fleet revision.
            let is_forgejo =
                serde_json::from_str::<shaula_core::fleet::FleetSpec>(&facts.spec_json)
                    .map(|spec| spec.kind == shaula_core::fleet::FleetProviderKind::Forgejo)
                    .unwrap_or(false);
            if !is_forgejo {
                self.store
                    .handoff_set_desired(&tx, &facts.resource_key, profile_key, resolved)
                    .await
                    .map_err(core_err)?;
                match self
                    .store
                    .fleet_auth_context_commit_tx(
                        &tx,
                        &facts.resource_key,
                        profile_key,
                        resolved,
                        &facts.spec_json,
                        facts.now,
                    )
                    .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(reason)) => {
                        let _ = tx.rollback().await;
                        return Ok(Err(MutationError::Unprocessable {
                            reason: if reason == "TargetNotAllowed" {
                                shaula_core::error::ReasonCode::TargetNotAllowed
                            } else {
                                shaula_core::error::ReasonCode::AmbiguousInstallation
                            },
                            summary: "auth profile target policy does not cover the fleet target"
                                .into(),
                        }));
                    }
                    Err(e) => return Err(core_err(e)),
                }
            }
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
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }

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
            // GitHub handoff remains cleanup-only after DELETE.
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
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }
}
