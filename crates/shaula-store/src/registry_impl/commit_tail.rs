//! The audit + outbox + optional idempotency tail shared by accepted
//! mutation commits. Callers own the transaction and its commit.

use shaula_core::error::CoreResult;
use shaula_core::registry::MutationFacts;

use super::{core_err, SqliteControlPlane};

impl SqliteControlPlane {
    /// Appends the accepted audit row (`kind`/`action`/`detail_json`),
    /// enqueues the outbox event under `kind`, and stores the idempotency
    /// record (if any) keyed `idem-<change id>`, all inside `tx`.
    pub(super) async fn commit_tail(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        facts: &MutationFacts,
        kind: &str,
        action: &str,
        detail_json: Option<String>,
    ) -> CoreResult<()> {
        self.store
            .audit_append(
                tx,
                shaula_core::registry::AuditAppend {
                    authentication: facts.authentication.clone(),
                    resource_kind: kind.into(),
                    action: action.into(),
                    actor: facts.actor.clone(),
                    resource_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    outcome: "accepted".into(),
                    detail_json,
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .outbox_enqueue(
                tx,
                kind,
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
        Ok(())
    }
}
