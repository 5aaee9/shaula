//! Fenced durable NO-OP commits for Template Profile, Fleet, and TemplatePool.

use sea_orm::EntityTrait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{AuditAppend, IdempotencyInsert, MutationError};

use super::{core_err, SqliteControlPlane};
use crate::entities::fleet::fleets;
use crate::entities::template::template_profiles;
use crate::entities::template_pool::template_pools;

impl SqliteControlPlane {
    /// Durable template-publish NO-OP (R9-02, spec 0005 §3): audit (+ the
    /// idempotent replay body) atomically, guarded by the SAME in-tx CAS
    /// as a real publish.
    pub(crate) async fn commit_template_noop_impl(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<(String, String, String)>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let current = template_profiles::Entity::find_by_id(key.to_string())
            .one(&tx)
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?
            .map(|c| (false, c.incarnation, c.desired_revision));
        if let Err(rejection) = noop_cas(key, current, incarnation, revision) {
            let _ = tx.rollback().await;
            return Ok(Err(rejection));
        }
        let idempotency = idempotency.map(|(idem, canonical, operation)| {
            let request_hash = shaula_core::auth::request_hash_parts(&[
                b"template_profile",
                key.as_bytes(),
                idem.as_bytes(),
                canonical.as_bytes(),
            ]);
            IdempotencyInsert {
                operation,
                principal: actor.name.clone(),
                id: format!("idem-noop-{}", shaula_core::auth::new_attempt_id()),
                resource_kind: "template_profile".to_string(),
                resource_key: key.to_string(),
                idempotency_key: idem,
                request_hash,
                response_status: 200,
                response_body: Some(
                    serde_json::json!({
                        "etag": format!("{incarnation}:{revision}"),
                        "change": {
                            "id": "",
                            "resource_kind": "template_profile",
                            "resource_key": key,
                            "revision": revision,
                            "kind": "NoOp",
                            "state": "NoOp",
                            "reason": null,
                        },
                        "no_op": true,
                    })
                    .to_string(),
                ),
                now,
            }
        });
        self.commit_noop_tail(
            &tx,
            ("template_profile", key, revision),
            actor,
            idempotency,
            now,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }

    pub(crate) async fn commit_fleet_noop_impl(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let current = fleets::Entity::find_by_id(key.to_string())
            .one(&tx)
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?
            .map(|c| {
                (
                    c.deletion_marker || c.tombstone,
                    c.incarnation,
                    c.desired_revision,
                )
            });
        if let Err(rejection) = noop_cas(key, current, incarnation, revision) {
            let _ = tx.rollback().await;
            return Ok(Err(rejection));
        }
        self.commit_noop_tail(&tx, ("fleet", key, revision), actor, idempotency, now)
            .await?;
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }

    pub(crate) async fn commit_template_pool_noop_impl(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let current = template_pools::Entity::find_by_id(key)
            .one(&tx)
            .await
            .map_err(|e| core_err(e.into()))?
            .map(|c| {
                (
                    c.deletion_marker || c.tombstone,
                    c.incarnation,
                    c.desired_revision,
                )
            });
        if let Err(rejection) = noop_cas(key, current, incarnation, revision) {
            let _ = tx.rollback().await;
            return Ok(Err(rejection));
        }
        self.commit_noop_tail(
            &tx,
            ("template_pool", key, revision),
            actor,
            idempotency,
            now,
        )
        .await?;
        tx.commit().await.map_err(|e| core_err(e.into()))?;
        Ok(Ok(()))
    }

    async fn commit_noop_tail(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        (resource_kind, key, revision): (&str, &str, i64),
        actor: &shaula_core::registry::Actor,
        idempotency: Option<IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<()> {
        self.store
            .audit_append(
                tx,
                AuditAppend {
                    authentication: actor.authentication.clone(),
                    resource_kind: resource_kind.to_string(),
                    action: "put".into(),
                    actor: actor.name.clone(),
                    resource_key: key.to_string(),
                    revision: Some(revision),
                    outcome: "noop".into(),
                    detail_json: None,
                    now,
                },
            )
            .await
            .map_err(core_err)?;
        if let Some(insert) = idempotency {
            self.store
                .idempotency_store(tx, insert)
                .await
                .map_err(core_err)?;
        }
        Ok(())
    }
}

/// The in-tx no-op CAS: `current` is `(deleted-or-tombstoned, incarnation,
/// desired_revision)`. Checks run in order NotFound, Gone, PreconditionFailed.
fn noop_cas(
    key: &str,
    current: Option<(bool, String, i64)>,
    incarnation: &str,
    revision: i64,
) -> Result<(), MutationError> {
    let Some((deleted, current_incarnation, current_revision)) = current else {
        return Err(MutationError::NotFound);
    };
    if deleted {
        return Err(MutationError::Gone {
            tombstone: key.to_string(),
        });
    }
    if current_incarnation != incarnation || current_revision != revision {
        return Err(MutationError::PreconditionFailed {
            current: (current_incarnation, current_revision),
        });
    }
    Ok(())
}
