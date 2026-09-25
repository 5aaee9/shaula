//! Shared TemplatePool Registry: conditional publication, reads and deletion.

use super::conditional_put::{pool::Pool, Request};
use super::{unprocessable, ControlPlane};
use async_trait::async_trait;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, ChangeView, MutationAccepted, MutationError, MutationFacts, Scope,
    TemplatePoolRegistryPort, TemplatePoolResource,
};
use shaula_core::template_pool::TemplatePoolSpec;

#[async_trait]
impl TemplatePoolRegistryPort for ControlPlane {
    #[tracing::instrument(name = "shaula.registry.template_pool_put", skip_all, fields(key = %key))]
    async fn template_pool_put(
        &self,
        actor: &Actor,
        key: &str,
        spec: TemplatePoolSpec,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.conditional_put(
            Request {
                actor,
                key,
                create: if_none_match,
                expected: if_match,
                idempotency_key,
            },
            || Pool::prepare(key, spec),
        )
        .await
    }

    async fn template_pool_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<TemplatePoolResource, MutationError>> {
        let Some(head) = self.store.template_pool_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if head.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let Some(latest) = self.store.template_pool_revision_latest(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let spec: TemplatePoolSpec = serde_json::from_str(&latest.spec_json)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        Ok(Ok(TemplatePoolResource {
            key: key.to_string(),
            spec,
            incarnation: head.incarnation,
            revision: head.desired_revision,
            resolved_members: latest.members,
            created_at: latest.created_at,
            updated_at: latest.created_at,
        }))
    }

    async fn template_pool_list(&self, _actor: &Actor) -> CoreResult<Vec<(String, i64, String)>> {
        self.store.template_pool_list().await
    }

    async fn template_pool_delete(
        &self,
        actor: &Actor,
        key: &str,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        if !actor.has(Scope::TemplateRetire) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.retire scope",
            )));
        }
        let precondition = match &if_match {
            Some((incarnation, revision)) => format!("if-match:{incarnation}:{revision}"),
            None => "none".to_string(),
        };
        match self
            .idempotency_replay(
                "template_pool",
                key,
                &idempotency_key,
                "delete",
                &precondition,
            )
            .await?
        {
            Err(conflict) => return Ok(Err(conflict)),
            Ok(Some(accepted)) => return Ok(Ok(accepted)),
            Ok(None) => {}
        }
        let Some((incarnation, revision)) = if_match else {
            return Ok(Err(MutationError::PreconditionRequired));
        };
        let Some(head) = self.store.template_pool_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if head.incarnation != incarnation || head.desired_revision != revision {
            return Ok(Err(MutationError::PreconditionFailed {
                current: (head.incarnation.clone(), head.desired_revision),
            }));
        }
        if head.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let now = self.now_ms();
        let change_id = self.new_id();
        let new_revision = head.desired_revision + 1;
        let change = ChangeView {
            id: change_id.clone(),
            resource_kind: "template_pool".into(),
            resource_key: key.to_string(),
            revision: new_revision,
            kind: "Delete".into(),
            state: "Pending".into(),
            reason: None,
        };
        let accepted = MutationAccepted {
            etag: format!("{incarnation}:{new_revision}"),
            change: change.clone(),
            no_op: false,
        };
        let idempotency = idempotency_key.map(|idem| {
            let request_hash =
                self.idempotency_hash("template_pool", key, &idem, "delete", &precondition);
            let response_body = serde_json::to_string(&accepted)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
            Ok((idem, request_hash, 202, response_body))
        });
        let idempotency = match idempotency {
            Some(Ok(value)) => Some(value),
            Some(Err(e)) => return Err(e),
            None => None,
        };
        let facts = MutationFacts {
            resource_kind: "template_pool",
            resource_key: key.to_string(),
            incarnation: head.incarnation.clone(),
            revision: new_revision,
            spec_json: "{}".to_string(),
            template: None,
            template_pool: Vec::new(),
            template_pool_ref: None,
            auth_desired: None,
            inputs_digest: "delete".to_string(),
            actor: actor.name.clone(),
            now,
            change,
            outbox_topic: "template_pool.change".to_string(),
            outbox_payload: format!("{{\"change\":\"{change_id}\"}}"),
            idempotency,
        };
        // The reference check remains INSIDE the tombstone transaction.
        if let Err(blocked) = self.store.commit_template_pool_delete(facts).await? {
            return Ok(Err(blocked));
        }
        Ok(Ok(accepted))
    }

    async fn template_pool_change_get(
        &self,
        _actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
        self.store.template_pool_change_get(change_id).await
    }
}
