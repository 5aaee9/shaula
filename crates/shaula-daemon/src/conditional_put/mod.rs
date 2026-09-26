//! Conditional publication sequencing shared by typed Registry adapters.
//!
//! Replay precedes mutable authority checks. Classification is provisional:
//! resource commits still own their transactional fences and effect gates.

use async_trait::async_trait;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{IdempotencyLookup, MutationAccepted, MutationError, Scope};

use super::{unprocessable, ControlPlane};
pub(super) use request::Request;
use request::{Commit, Head, Identity, Plan};

pub(super) mod auth;
pub(super) mod fleet;
pub(super) mod pool;
mod request;
pub(super) mod template;

pub(super) type Outcome<T> = CoreResult<Result<T, MutationError>>;

/// Internal resource seam: protocol ordering is deliberately not a hook.
#[async_trait]
pub(super) trait Resource: Send + Sync {
    type Head: Head;
    type Facts: Send;
    const SCOPE: Scope;

    fn operation(&self) -> &'static str {
        "v1:PUT"
    }

    fn identity(&self) -> Identity<'_>;
    async fn current(&self, plane: &ControlPlane, key: &str) -> CoreResult<Option<Self::Head>>;
    async fn admit(
        &self,
        plane: &ControlPlane,
        key: &str,
        head: Option<&Self::Head>,
    ) -> Outcome<Plan<Self::Facts>>;
    async fn commit(
        &self,
        plane: &ControlPlane,
        commit: Commit<'_>,
        facts: Option<Self::Facts>,
    ) -> Outcome<()>;

    /// Non-secret resources need no additional immutable-material comparison.
    async fn replay_matches(
        &self,
        _plane: &ControlPlane,
        _key: &str,
        _accepted: &MutationAccepted,
    ) -> CoreResult<bool> {
        Ok(true)
    }
}

impl ControlPlane {
    pub(super) async fn conditional_put<R: Resource>(
        &self,
        request: Request<'_>,
        prepare: impl FnOnce() -> Outcome<R> + Send,
    ) -> Outcome<MutationAccepted> {
        // Never query historical or current state before authorization. Resource
        // format validation also precedes replay (notably retired Auth formats).
        if !request.actor.has(R::SCOPE) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                format!("missing {} scope", R::SCOPE.as_str()),
            )));
        }
        let resource = match prepare()? {
            Ok(resource) => resource,
            Err(error) => return Ok(Err(error)),
        };
        match self.publication_replay(&request, &resource).await? {
            Ok(Some(accepted)) => return Ok(Ok(accepted)),
            Err(error) => return Ok(Err(error)),
            Ok(None) => {}
        }
        // Preserve old hash bytes and accepted results, but do not admit a
        // new ambiguous request. HTTP must not reject this before replay.
        if request.create && request.expected.is_some() {
            return Ok(Err(MutationError::ConflictingPreconditions));
        }
        let outcome = self.publish_once(&request, &resource).await;
        if request.idempotency_key.is_some()
            && matches!(
                &outcome,
                Ok(Err(MutationError::PreconditionFailed { .. })) | Err(_)
            )
        {
            // Both callers may have read an idempotency miss. A CAS/unique-key
            // loser can read the winner, never recompute or retry a mutation.
            match self.publication_replay(&request, &resource).await? {
                Ok(Some(accepted)) => return Ok(Ok(accepted)),
                Err(error) => return Ok(Err(error)),
                Ok(None) => {}
            }
        }
        outcome
    }

    async fn publication_replay<R: Resource>(
        &self,
        request: &Request<'_>,
        resource: &R,
    ) -> Outcome<Option<MutationAccepted>> {
        let Some(idem) = &request.idempotency_key else {
            return Ok(Ok(None));
        };
        let identity = resource.identity();
        let hash = identity.hash(request, idem);
        match self
            .store
            .idempotency_find(
                &request.actor.name,
                resource.operation(),
                identity.kind,
                request.key,
                idem,
                &hash,
            )
            .await?
        {
            IdempotencyLookup::LegacyConflict => Ok(Err(MutationError::LegacyIdempotencyConflict)),
            IdempotencyLookup::Miss => Ok(Ok(None)),
            IdempotencyLookup::Conflict => Ok(Err(MutationError::IdempotencyConflict)),
            IdempotencyLookup::Replay(body) => {
                let accepted = serde_json::from_str(&body).map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "stored publication result is invalid")
                })?;
                if resource
                    .replay_matches(self, request.key, &accepted)
                    .await?
                {
                    Ok(Ok(Some(accepted)))
                } else {
                    Ok(Err(MutationError::IdempotencyConflict))
                }
            }
        }
    }

    async fn publish_once<R: Resource>(
        &self,
        request: &Request<'_>,
        resource: &R,
    ) -> Outcome<MutationAccepted> {
        let head = resource.current(self, request.key).await?;
        if let Some(head) = &head {
            if let Err(error) = head.writable() {
                return Ok(Err(error));
            }
        }
        let version = head.as_ref().map(Head::version);
        if let Err(error) = super::profile_conditions::check(
            version
                .as_ref()
                .map(|(incarnation, revision)| (incarnation.as_str(), *revision)),
            request.create,
            request.expected.as_ref(),
        ) {
            return Ok(Err(error));
        }
        let plan = match resource.admit(self, request.key, head.as_ref()).await? {
            Ok(plan) => plan,
            Err(error) => return Ok(Err(error)),
        };
        let identity = resource.identity();
        let (accepted, incarnation, facts) = plan.accept(identity.kind, request.key, version)?;
        let commit = Commit {
            operation: resource.operation(),
            key: request.key,
            actor: request.actor,
            canonical: identity.canonical,
            incarnation: &incarnation,
            idempotency: identity.record(request, &accepted)?,
            accepted: &accepted,
            now: self.now_ms(),
        };
        match resource.commit(self, commit, facts).await? {
            Ok(()) => Ok(Ok(accepted)),
            Err(error) => Ok(Err(error)),
        }
    }
}
