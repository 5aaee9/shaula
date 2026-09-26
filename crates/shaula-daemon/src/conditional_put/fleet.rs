//! Fleet admission retains exact pins, identity and occupancy rules.

use async_trait::async_trait;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::{normalize_fleet, validate_fleet_spec, FleetProviderKind, FleetSpec};
use shaula_core::registry::{FleetHead, MutationError, Scope};
use shaula_core::template_pool::{FleetPoolRef, ResolvedTemplatePoolMember};

use super::request::{Commit, Identity, Plan};
use super::{ControlPlane, Outcome, Resource};
use crate::service::{template_referenced, unprocessable};

pub(in crate::service) struct Fleet {
    spec: FleetSpec,
    canonical: String,
}

pub(in crate::service) struct Admitted {
    template: Option<(String, i64, String, String)>,
    members: Vec<ResolvedTemplatePoolMember>,
    pool: Option<FleetPoolRef>,
    auth: (String, i64),
    inputs_digest: String,
}

impl Fleet {
    pub(in crate::service) fn prepare(spec: FleetSpec) -> Outcome<Self> {
        if let Err(error) = validate_fleet_spec(&spec) {
            return Ok(Err(unprocessable(error.code, error.summary)));
        }
        let canonical = serde_json::to_string(&spec)
            .map_err(|error| CoreError::new(ReasonCode::Internal, error.to_string()))?;
        Ok(Ok(Self { spec, canonical }))
    }

    async fn replacement_allowed(&self, plane: &ControlPlane, key: &str) -> Outcome<()> {
        let Some(previous) = plane.store.fleet_revision_latest(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let previous_spec: FleetSpec = serde_json::from_str(&previous.spec_json)
            .map_err(|error| CoreError::new(ReasonCode::Internal, error.to_string()))?;
        let spec = &self.spec;
        let identity_changed = match (previous_spec.kind, spec.kind) {
            (FleetProviderKind::Github, FleetProviderKind::Github) => {
                previous_spec.github.target != spec.github.target
                    || previous_spec.github.scale_set_name != spec.github.scale_set_name
                    || previous_spec.github.runner_group != spec.github.runner_group
            }
            (FleetProviderKind::Forgejo, FleetProviderKind::Forgejo) => {
                previous_spec.forgejo != spec.forgejo
            }
            _ => true,
        };
        if identity_changed {
            return Ok(Err(MutationError::IdentityConflict));
        }
        let pool_changed = previous_spec.template_pool != spec.template_pool;
        let pool_ref_changed = previous_spec.template_pool_ref != spec.template_pool_ref;
        if (template_referenced(&previous_spec) != template_referenced(spec)
            || previous_spec.template_inputs != spec.template_inputs
            || pool_changed
            || pool_ref_changed)
            && plane.store.generations_occupancy(key).await? > 0
        {
            return Ok(Err(MutationError::RetirementBlocked {
                reason: if previous_spec.template_inputs != spec.template_inputs {
                    "template input replacement requires zero resource occupancy".into()
                } else if pool_changed || pool_ref_changed {
                    "template pool replacement requires zero resource occupancy".into()
                } else {
                    "replacement requires zero resource occupancy".into()
                },
            }));
        }
        if previous.auth_desired.0 != spec.auth_profile_ref()
            && plane.store.generations_occupancy(key).await? > 0
        {
            return Ok(Err(MutationError::RetirementBlocked {
                reason: "auth profile replacement requires zero resource occupancy".into(),
            }));
        }
        Ok(Ok(()))
    }
}

#[async_trait]
impl Resource for Fleet {
    type Head = FleetHead;
    type Facts = Admitted;
    const SCOPE: Scope = Scope::FleetWrite;

    fn identity(&self) -> Identity<'_> {
        Identity {
            kind: "fleet",
            canonical: &self.canonical,
            includes_precondition: true,
        }
    }

    async fn current(&self, plane: &ControlPlane, key: &str) -> CoreResult<Option<FleetHead>> {
        plane.store.fleet_get(key).await
    }

    async fn admit(
        &self,
        plane: &ControlPlane,
        key: &str,
        head: Option<&FleetHead>,
    ) -> Outcome<Plan<Admitted>> {
        if head.is_none() && plane.store.fleet_count().await? >= plane.max_active_fleets {
            return Ok(Err(MutationError::TooManyRequests {
                retry_after_secs: 5,
            }));
        }
        let (template, members, pool, auth) =
            match plane.resolve_admission_materials(key, &self.spec).await? {
                Ok(materials) => materials,
                Err(error) => return Ok(Err(error)),
            };
        if head.is_some() {
            if let Err(error) = self.replacement_allowed(plane, key).await? {
                return Ok(Err(error));
            }
        }
        // Same-Profile Auth promotion must not advance the Fleet ETag.
        if plane
            .detect_noop(key, head, &self.spec, template.as_ref())
            .await?
            .is_some()
        {
            return Ok(Ok(Plan::NoOp));
        }
        let normalized = normalize_fleet(&self.spec, plane.inputs_digest(&self.spec)?)?;
        Ok(Ok(Plan::Revision {
            kind: if head.is_some() { "Replace" } else { "Create" },
            facts: Admitted {
                template,
                members,
                pool,
                auth: (auth.profile_key.as_str().into(), auth.revision as i64),
                inputs_digest: normalized.inputs_digest,
            },
        }))
    }

    async fn commit(
        &self,
        plane: &ControlPlane,
        commit: Commit<'_>,
        admitted: Option<Admitted>,
    ) -> Outcome<()> {
        let Some(admitted) = admitted else {
            return plane
                .store
                .commit_fleet_noop(
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
            "fleet",
            "fleet.change",
            format!("{{\"change\":\"{}\"}}", commit.accepted.change.id),
        );
        facts.spec_json = self.canonical.clone();
        facts.template = admitted.template;
        facts.template_pool = admitted.members;
        facts.template_pool_ref = admitted.pool;
        facts.auth_desired = Some(admitted.auth);
        facts.inputs_digest = admitted.inputs_digest;
        // Keep the short exclusive effect gate and ALL in-transaction checks.
        // Protocol reuse does not serialize pool or profile writes on this gate.
        let gate = plane.effect_gates.acquire_exclusive(commit.key).await;
        let result = plane.store.commit_fleet_mutation(facts).await;
        drop(gate);
        crate::service::metrics::record_admission(&result);
        result
    }
}
