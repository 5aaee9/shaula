//! Pool publication always resolves current Active member pins.

use async_trait::async_trait;
use shaula_core::auth::is_stable_identifier;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::Scope;
use shaula_core::template_pool::{ResolvedTemplatePoolMember, TemplatePoolHead, TemplatePoolSpec};

use super::request::{Commit, Identity, Plan};
use super::{ControlPlane, Outcome, Resource};
use crate::service::{sha256_digest, template_inputs_digest, unprocessable, validate_inputs};

pub(in crate::service) struct Pool {
    spec: TemplatePoolSpec,
    canonical: String,
}

impl Pool {
    pub(in crate::service) fn prepare(key: &str, spec: TemplatePoolSpec) -> Outcome<Self> {
        if !is_stable_identifier(key) || key.len() > 128 {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "template pool key must be a stable identifier of at most 128 characters",
            )));
        }
        if let Err(error) = spec.validate() {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        let canonical = serde_json::to_string(&spec)
            .map_err(|error| CoreError::new(ReasonCode::Internal, error.to_string()))?;
        Ok(Ok(Self { spec, canonical }))
    }
}

#[async_trait]
impl Resource for Pool {
    type Head = TemplatePoolHead;
    type Facts = Vec<ResolvedTemplatePoolMember>;
    const SCOPE: Scope = Scope::TemplatePublish;

    fn identity(&self) -> Identity<'_> {
        Identity {
            kind: "template_pool",
            canonical: &self.canonical,
            includes_precondition: true,
        }
    }
    async fn current(
        &self,
        plane: &ControlPlane,
        key: &str,
    ) -> CoreResult<Option<TemplatePoolHead>> {
        plane.store.template_pool_get(key).await
    }
    async fn admit(
        &self,
        plane: &ControlPlane,
        key: &str,
        head: Option<&TemplatePoolHead>,
    ) -> Outcome<Plan<Self::Facts>> {
        let members = match plane.resolve_pool_members(&self.spec).await? {
            Ok(members) => members,
            Err(error) => return Ok(Err(error)),
        };
        if head.is_some() {
            if let Some(latest) = plane.store.template_pool_revision_latest(key).await? {
                let same_members = latest.members.len() == members.len()
                    && latest.members.iter().zip(&members).all(|(a, b)| {
                        a.key == b.key
                            && a.template_profile_key == b.template_profile_key
                            && a.template_revision == b.template_revision
                            && a.template_artifact_digest == b.template_artifact_digest
                            && a.template_attestation_id == b.template_attestation_id
                            && a.inputs_digest == b.inputs_digest
                            && a.weight == b.weight
                            && a.max_runners == b.max_runners
                    });
                if latest.spec_json == self.canonical && same_members {
                    return Ok(Ok(Plan::NoOp));
                }
            }
        }
        Ok(Ok(Plan::Revision {
            kind: if head.is_some() { "Replace" } else { "Create" },
            facts: members,
        }))
    }
    async fn commit(
        &self,
        plane: &ControlPlane,
        commit: Commit<'_>,
        members: Option<Self::Facts>,
    ) -> Outcome<()> {
        let Some(members) = members else {
            return plane
                .store
                .commit_template_pool_noop(
                    commit.key,
                    commit.incarnation,
                    commit.revision(),
                    commit.actor,
                    commit.noop_record(),
                    commit.now,
                )
                .await;
        };
        let mut facts = commit.facts(
            "template_pool",
            "template_pool.change",
            format!("{{\"change\":\"{}\"}}", commit.accepted.change.id),
        );
        facts.spec_json = self.canonical.clone();
        facts.template_pool = members;
        facts.inputs_digest = sha256_digest(self.canonical.as_bytes());
        // Pools own no runners and do not acquire a Fleet effect gate.
        plane.store.commit_template_pool_mutation(facts).await
    }
}

impl ControlPlane {
    /// Shared by explicit PUT and cascade; both resolve current Active with
    /// the profile policy AND artifact schema as independent authorities.
    pub(crate) async fn resolve_pool_members(
        &self,
        spec: &TemplatePoolSpec,
    ) -> Outcome<Vec<ResolvedTemplatePoolMember>> {
        let mut resolved = Vec::with_capacity(spec.members.len());
        for member in &spec.members {
            let pin = match self
                .resolve_template_ref(&member.template_profile_ref)
                .await
            {
                Ok(pin) => pin,
                Err(error) => return Ok(Err(unprocessable(error.code, error.summary))),
            };
            let policy = self
                .store
                .template_revision_get(&pin.0, pin.1)
                .await?
                .and_then(|r| r.fleet_input_policy_json)
                .unwrap_or_else(|| "{}".into());
            let schema = self.store.artifact_parameter_schema(&pin.2).await?;
            if schema.trim().is_empty() {
                return Err(CoreError::new(
                    ReasonCode::StorageUnavailable,
                    "pool member artifact has a blank parameter schema document",
                ));
            }
            if let Err(error) = validate_inputs(&member.template_inputs, &policy, Some(&schema)) {
                return Ok(Err(unprocessable(error.code, error.summary)));
            }
            let inputs_digest = template_inputs_digest(&member.template_inputs)?;
            resolved.push(ResolvedTemplatePoolMember {
                key: member.key.clone(),
                template_profile_key: pin.0,
                template_revision: pin.1,
                template_artifact_digest: pin.2,
                template_attestation_id: pin.3,
                template_inputs: member.template_inputs.clone(),
                inputs_digest,
                weight: member.weight,
                max_runners: member.max_runners,
            });
        }
        Ok(Ok(resolved))
    }
}
