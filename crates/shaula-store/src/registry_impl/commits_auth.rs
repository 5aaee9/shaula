//! One Auth Candidate writer for full credentials and explicit policy updates.

use shaula_core::error::CoreResult;
use shaula_core::registry::{AuthRevisionRow, MutationError, MutationFacts};

use super::{core_err, SqliteControlPlane};

impl SqliteControlPlane {
    pub(crate) async fn commit_auth_revision_impl(
        &self,
        facts: MutationFacts,
        credential: AuthRevisionRow,
        secret_bytes: &[u8],
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        self.commit_auth_candidate(tx, facts, credential, secret_bytes, None)
            .await
    }

    pub(crate) async fn commit_auth_candidate(
        &self,
        tx: sea_orm::DatabaseTransaction,
        facts: MutationFacts,
        credential: AuthRevisionRow,
        secret_bytes: &[u8],
        base_revision: Option<i64>,
    ) -> CoreResult<Result<(), MutationError>> {
        if let Err(error) = self
            .store
            .auth_commit_revision(
                &tx,
                shaula_core::auth::AuthRevisionInsert {
                    key: facts.resource_key.clone(),
                    incarnation: facts.incarnation.clone(),
                    revision: facts.revision,
                    kind: credential.kind.clone(),
                    app_id: credential.app_id.clone(),
                    schema_version: credential.schema_version,
                    policy_json: credential.policy_json.clone(),
                },
                secret_bytes,
                facts.now,
            )
            .await
        {
            let _ = tx.rollback().await;
            if matches!(error, crate::store::StoreError::Conflict { .. }) {
                let current = self
                    .store
                    .auth_profile_get(&facts.resource_key)
                    .await
                    .map_err(core_err)?;
                return Ok(Err(match current {
                    Some(head) => MutationError::PreconditionFailed {
                        current: (head.incarnation, head.desired_revision),
                    },
                    None => MutationError::NotFound,
                }));
            }
            return Err(core_err(error));
        }
        self.store
            .profile_change_insert(
                &tx,
                shaula_core::registry::ProfileChangeInsert {
                    id: facts.change.id.clone(),
                    resource_kind: "github_auth_profile".into(),
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
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: "github_auth_profile".into(),
                    action: if base_revision.is_some() {
                        "policy_update"
                    } else {
                        "put"
                    }
                    .into(),
                    actor: facts.actor.clone(),
                    resource_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    outcome: "accepted".into(),
                    detail_json: base_revision
                        .map(|base| serde_json::json!({"base_revision": base}).to_string()),
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .outbox_enqueue(
                &tx,
                "github_auth_profile",
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
