use super::ControlPlane;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, ChangeView, IdempotencyLookup, MutationAccepted, MutationError, MutationFacts,
};

impl ControlPlane {
    pub(super) async fn retire_profile(
        &self,
        actor: &Actor,
        key: &str,
        template: bool,
        expected: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        let kind = if template {
            "template_profile"
        } else {
            "github_auth_profile"
        };
        let hash =
            shaula_core::auth::request_hash_parts(&[kind.as_bytes(), key.as_bytes(), b"DELETE"]);
        if let Some(idem) = &idempotency_key {
            match self.store.idempotency_find(kind, key, idem, &hash).await? {
                IdempotencyLookup::Replay(body) => {
                    return serde_json::from_str(&body).map(Ok).map_err(|_| {
                        CoreError::new(ReasonCode::Internal, "invalid retirement replay")
                    })
                }
                IdempotencyLookup::Conflict => return Ok(Err(MutationError::IdempotencyConflict)),
                IdempotencyLookup::Miss => {}
            }
        }
        let head = if template {
            self.store.template_profile_get(key).await?
        } else {
            self.store.auth_profile_get(key).await?
        };
        let Some(head) = head else {
            return Ok(Err(MutationError::NotFound));
        };
        if let Err(error) = super::profile_conditions::check(
            Some((&head.incarnation, head.desired_revision)),
            false,
            expected.as_ref(),
        ) {
            return Ok(Err(error));
        }
        let now = self.now_ms();
        // Existing heads and exact pins are retained until retention/reference clearance.
        let accepted = MutationAccepted {
            etag: format!("{}:{}", head.incarnation, head.desired_revision),
            no_op: false,
            change: ChangeView {
                id: self.new_id(),
                resource_kind: kind.into(),
                resource_key: key.into(),
                revision: head.desired_revision,
                kind: "Retire".into(),
                state: "Blocked".into(),
                reason: Some("ResourceInUse".into()),
            },
        };
        let body = serde_json::to_string(&accepted)
            .map_err(|_| CoreError::new(ReasonCode::Internal, "retirement serialization failed"))?;
        let facts = MutationFacts {
            resource_kind: kind,
            resource_key: key.into(),
            incarnation: head.incarnation,
            revision: head.desired_revision,
            spec_json: String::new(),
            template: None,
            auth_desired: None,
            inputs_digest: String::new(),
            actor: actor.name.clone(),
            now,
            change: accepted.change.clone(),
            outbox_topic: "profile.retire".into(),
            outbox_payload: serde_json::json!({"kind": kind, "key": key}).to_string(),
            idempotency: idempotency_key.map(|k| (k, hash, 202, body)),
        };
        match self.store.commit_profile_retirement(facts).await? {
            Ok(()) => Ok(Ok(accepted)),
            Err(e) => Ok(Err(e)),
        }
    }
}
