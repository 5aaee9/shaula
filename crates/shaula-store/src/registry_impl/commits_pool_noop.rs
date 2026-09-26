//! A pool no-op is a fenced durable reassertion, not an early return.

use sea_orm::EntityTrait;
use shaula_core::error::CoreResult;
use shaula_core::registry::{AuditAppend, IdempotencyInsert, MutationError};

use super::{core_err, SqliteControlPlane};
use crate::entities::template_pool::template_pools;

impl SqliteControlPlane {
    pub(crate) async fn commit_template_pool_noop_impl(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        // Reserve the writer before reading the authority. PUT, cascade and
        // DELETE cannot change the head between this check and the audit.
        let tx = self.store.begin().await.map_err(core_err)?;
        let Some(current) = template_pools::Entity::find_by_id(key)
            .one(&tx)
            .await
            .map_err(|error| core_err(error.into()))?
        else {
            return Ok(Err(MutationError::NotFound));
        };
        if current.deletion_marker || current.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.into(),
            }));
        }
        if current.incarnation != incarnation || current.desired_revision != revision {
            return Ok(Err(MutationError::PreconditionFailed {
                current: (current.incarnation, current.desired_revision),
            }));
        }
        self.store
            .audit_append(
                &tx,
                AuditAppend {
                    resource_kind: "template_pool".into(),
                    action: "put".into(),
                    actor: actor.name.clone(),
                    authentication: actor.authentication.clone(),
                    resource_key: key.into(),
                    revision: Some(revision),
                    outcome: "noop".into(),
                    detail_json: None,
                    now,
                },
            )
            .await
            .map_err(core_err)?;
        if let Some(record) = idempotency {
            self.store
                .idempotency_store(&tx, record)
                .await
                .map_err(core_err)?;
        }
        tx.commit().await.map_err(|error| core_err(error.into()))?;
        Ok(Ok(()))
    }
}
