//! NO-OP commit implementations (R9-02), split to keep commits.rs
//! within the 400-line limit (AGENTS.md).

use shaula_core::error::CoreResult;
use shaula_core::registry::MutationError;

use sea_orm::EntityTrait;

use crate::entities::template::template_profiles;

use super::core_err;
use super::SqliteControlPlane;

impl SqliteControlPlane {
    /// Durable template-publish NO-OP (R9-02, spec 0005 §3): audit (+ the
    /// idempotent replay body) atomically, guarded by the SAME in-tx CAS
    /// as a real publish — a concurrent PUT that advanced the head turns
    /// the stale no-op into a precondition failure.
    pub(crate) async fn commit_template_noop_impl(
        &self,
        key: &str,
        incarnation: &str,
        revision: i64,
        actor: &str,
        idempotency: Option<(String, String)>,
        now: i64,
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        let Some(current) = template_profiles::Entity::find_by_id(key.to_string())
            .one(&tx)
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?
        else {
            let _ = tx.rollback().await;
            return Ok(Err(MutationError::NotFound));
        };
        if current.incarnation != incarnation || current.desired_revision != revision {
            let _ = tx.rollback().await;
            return Ok(Err(MutationError::PreconditionFailed {
                current: (current.incarnation.clone(), current.desired_revision),
            }));
        }
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: "template_profile".into(),
                    action: "put".into(),
                    actor: actor.to_string(),
                    resource_key: key.to_string(),
                    revision: Some(revision),
                    outcome: "noop".into(),
                    detail_json: None,
                    now,
                },
            )
            .await
            .map_err(core_err)?;
        if let Some((idem, canonical)) = idempotency {
            let request_hash = shaula_core::auth::request_hash_parts(&[
                b"template_profile",
                key.as_bytes(),
                idem.as_bytes(),
                canonical.as_bytes(),
            ]);
            self.store
                .idempotency_store(
                    &tx,
                    shaula_core::registry::IdempotencyInsert {
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
