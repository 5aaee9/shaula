use shaula_core::auth::new_attempt_id;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{
    Actor, ChangeView, FleetHead, IdempotencyInsert, MutationAccepted, MutationError,
    MutationFacts, ProfileHead,
};
use shaula_core::template_pool::TemplatePoolHead;

pub(in crate::service) struct Request<'a> {
    pub actor: &'a Actor,
    pub key: &'a str,
    pub create: bool,
    pub expected: Option<(String, i64)>,
    pub idempotency_key: Option<String>,
}

pub(in crate::service) struct Identity<'a> {
    pub kind: &'static str,
    pub canonical: &'a str,
    // These are existing durable formats, not optional policy knobs.
    pub includes_precondition: bool,
}

impl Identity<'_> {
    pub fn hash(&self, request: &Request<'_>, idem: &str) -> String {
        let mut parts = vec![
            self.kind.as_bytes(),
            request.key.as_bytes(),
            idem.as_bytes(),
            self.canonical.as_bytes(),
        ];
        let condition;
        if self.includes_precondition {
            condition = match (&request.expected, request.create) {
                (Some((incarnation, revision)), _) => format!("if-match:{incarnation}:{revision}"),
                (None, true) => "if-none-match:*".into(),
                (None, false) => "none".into(),
            };
            parts.push(condition.as_bytes());
        }
        shaula_core::auth::request_hash_parts(&parts)
    }

    pub fn record(
        &self,
        request: &Request<'_>,
        accepted: &MutationAccepted,
    ) -> CoreResult<Option<(String, String, i32, String)>> {
        request
            .idempotency_key
            .as_ref()
            .map(|idem| {
                Ok((
                    idem.clone(),
                    self.hash(request, idem),
                    if accepted.no_op { 200 } else { 202 },
                    serde_json::to_string(accepted)
                        .map_err(|_| internal("publication result encoding failed"))?,
                ))
            })
            .transpose()
    }
}

pub(in crate::service) trait Head: Send + Sync {
    fn version(&self) -> (String, i64);
    fn writable(&self) -> Result<(), MutationError>;
}

impl Head for FleetHead {
    fn version(&self) -> (String, i64) {
        (self.incarnation.clone(), self.desired_revision)
    }
    fn writable(&self) -> Result<(), MutationError> {
        if self.deletion_marker || self.tombstone {
            Err(MutationError::Gone {
                tombstone: self.key.clone(),
            })
        } else {
            Ok(())
        }
    }
}
impl Head for TemplatePoolHead {
    fn version(&self) -> (String, i64) {
        (self.incarnation.clone(), self.desired_revision)
    }
    fn writable(&self) -> Result<(), MutationError> {
        if self.deletion_marker || self.tombstone {
            Err(MutationError::Gone {
                tombstone: self.key.clone(),
            })
        } else {
            Ok(())
        }
    }
}
impl Head for ProfileHead {
    fn version(&self) -> (String, i64) {
        (self.incarnation.clone(), self.desired_revision)
    }
    fn writable(&self) -> Result<(), MutationError> {
        if matches!(self.status.as_str(), "Retiring" | "Retired") {
            Err(MutationError::RetirementBlocked {
                reason: "profile is retiring".into(),
            })
        } else {
            Ok(())
        }
    }
}

pub(in crate::service) enum Plan<T> {
    NoOp,
    Revision { kind: &'static str, facts: T },
}

impl<T> Plan<T> {
    pub fn accept(
        self,
        resource_kind: &str,
        key: &str,
        current: Option<(String, i64)>,
    ) -> CoreResult<(MutationAccepted, String, Option<T>)> {
        let (incarnation, revision, kind, facts, no_op) = match self {
            Self::NoOp => {
                let (incarnation, revision) =
                    current.ok_or_else(|| internal("no-op requires a current revision"))?;
                (incarnation, revision, "NoOp", None, true)
            }
            Self::Revision { kind, facts } => {
                let (incarnation, revision) = match current {
                    Some((incarnation, revision)) => (
                        incarnation,
                        revision
                            .checked_add(1)
                            .ok_or_else(|| internal("publication revision exhausted"))?,
                    ),
                    None => (new_attempt_id(), 1),
                };
                (incarnation, revision, kind, Some(facts), false)
            }
        };
        Ok((
            MutationAccepted {
                etag: format!("{incarnation}:{revision}"),
                change: ChangeView {
                    id: if no_op {
                        String::new()
                    } else {
                        new_attempt_id()
                    },
                    resource_kind: resource_kind.into(),
                    resource_key: key.into(),
                    revision,
                    kind: kind.into(),
                    state: if no_op { "NoOp" } else { "Pending" }.into(),
                    reason: None,
                },
                no_op,
            },
            incarnation,
            facts,
        ))
    }
}

pub(in crate::service) struct Commit<'a> {
    pub operation: &'static str,
    pub key: &'a str,
    pub actor: &'a Actor,
    pub canonical: &'a str,
    pub incarnation: &'a str,
    pub accepted: &'a MutationAccepted,
    pub idempotency: Option<(String, String, i32, String)>,
    pub now: i64,
}

impl Commit<'_> {
    pub fn revision(&self) -> i64 {
        self.accepted.change.revision
    }

    /// Preserve the existing no-op storage record shape.
    pub fn noop_record(&self) -> Option<IdempotencyInsert> {
        self.idempotency
            .as_ref()
            .map(|(key, hash, status, body)| IdempotencyInsert {
                operation: self.operation.into(),
                principal: self.actor.name.clone(),
                id: format!("idem-noop-{}", new_attempt_id()),
                resource_kind: self.accepted.change.resource_kind.clone(),
                resource_key: self.key.into(),
                idempotency_key: key.clone(),
                request_hash: hash.clone(),
                response_status: *status,
                response_body: Some(body.clone()),
                now: self.now,
            })
    }

    /// Only common protocol facts live here. Resource adapters still own
    /// the legacy overloaded fields; changing their shape is a separate task.
    pub fn facts(&self, kind: &'static str, topic: &str, payload: String) -> MutationFacts {
        MutationFacts {
            idempotency_operation: self.operation,
            authentication: self.actor.authentication.clone(),
            resource_kind: kind,
            resource_key: self.key.into(),
            incarnation: self.incarnation.into(),
            revision: self.revision(),
            spec_json: String::new(),
            template: None,
            template_pool: Vec::new(),
            template_pool_ref: None,
            auth_desired: None,
            inputs_digest: String::new(),
            actor: self.actor.name.clone(),
            now: self.now,
            change: self.accepted.change.clone(),
            outbox_topic: topic.into(),
            outbox_payload: payload,
            idempotency: self.idempotency.clone(),
        }
    }
}

pub(super) fn internal(summary: &str) -> CoreError {
    CoreError::new(ReasonCode::Internal, summary)
}
