//! Idempotent no-op Fleet writes still compare the caller's revision.
use super::{core_err, SqliteControlPlane};
use crate::entities::fleet::fleets;
use sea_orm::EntityTrait;
use shaula_core::error::CoreResult;
use shaula_core::registry::MutationError;

impl SqliteControlPlane {
    pub(crate) async fn commit_fleet_noop_impl(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &shaula_core::registry::Actor,
        idempotency: Option<shaula_core::registry::IdempotencyInsert>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        // In-tx CAS against the caller's read (spec 0002 §5.3): a newer
        // PUT or a DELETE that landed since the no-op was classified must
        // never have its precondition silently ignored — the no-op record
        // would then pin a stale (incarnation, revision).
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
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    authentication: actor.authentication.clone(),
                    resource_kind: "fleet".to_string(),
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
                .idempotency_store(&tx, insert)
                .await
                .map_err(core_err)?;
        }
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }
}

/// The in-tx no-op CAS shared by Fleet and TemplatePool: `current` is the
/// head as (deleted-or-tombstoned, incarnation, desired revision). Checks
/// run in order NotFound, Gone, PreconditionFailed.
pub(super) fn noop_cas(
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
