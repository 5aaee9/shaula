//! Follow-latest template cascade (spec 0023): every live fleet follows
//! its profile's Active revision (follow-only since ARD-0029). The
//! level-triggered scan mints a replacement revision — unchanged spec,
//! new resolved pin — on the next pass, occupied or not: live Generations
//! keep their own pins, so only new Creates use the new revision (a shared
//! pool reference likewise, spec 0037 §5). Failures
//! degrade to per-fleet WARNs and retry next tick; the scan loop itself
//! never fails from one fleet's state.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetSpec;
use shaula_core::registry::{Actor, ChangeView, MutationError, MutationFacts, Scope};

use super::ControlPlane;
use shaula_core::diagnostics::*;

#[path = "service_follow_pool.rs"]
mod pool;

struct FollowerTarget<'a> {
    key: &'a str,
    head: &'a shaula_core::registry::FleetHead,
    latest: &'a shaula_core::registry::store_port::FleetRevisionRow,
}

impl ControlPlane {
    /// The cascade acts with the daemon's own service identity; its audit
    /// records stay distinguishable from operator mutations.
    fn daemon_actor() -> Actor {
        Actor {
            authentication: Default::default(),
            name: "shaula-daemon".to_string(),
            scopes: vec![
                Scope::FleetRead,
                Scope::FleetWrite,
                Scope::TemplateRead,
                Scope::AuthRead,
            ],
        }
    }

    /// One cascade pass over all live fleets; returns how many followers
    /// were upgraded this tick.
    pub async fn cascade_template_follow_upgrades(&self, now: i64) -> CoreResult<u32> {
        let mut upgraded = 0u32;
        // Shared pools re-resolve FIRST (spec 0037 §5): a member template's
        // new Active mints one new pool revision; referencing fleets catch
        // up in the fleet pass below.
        upgraded += self.cascade_pool_follow_upgrades(now).await?;
        for (key, _revision, _phase) in self
            .store
            .fleet_list(&Self::daemon_actor(), Default::default())
            .await?
        {
            match self.maybe_upgrade_follower(&key, now).await {
                Ok(true) => upgraded += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(fleet = %key, summary = %error.summary, "template follow upgrade failed");
                }
            }
        }
        Ok(upgraded)
    }

    async fn maybe_upgrade_follower(&self, key: &str, now: i64) -> CoreResult<bool> {
        let Some(head) = self.store.fleet_get(key).await? else {
            return Ok(false);
        };
        let mut diagnostic = Capture::start(
            self.store.diagnostic_sink(),
            Guard::fleet(key, &shaula_core::registry::FleetRuntimeGuard::from(&head)),
            Lane::Follow,
            QuestionId::Rollout,
            now,
        );
        // Decommissioning fleets converge through their own cleanup path.
        if head.tombstone || head.deletion_marker {
            return Ok(false);
        }
        let Some(latest) = self.store.fleet_revision_latest(key).await? else {
            return Ok(false);
        };
        let spec: FleetSpec = serde_json::from_str(&latest.spec_json)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        // Pool members own independent follow/pin state. The pool admission
        // contract retains each member's exact revision, so the single
        // template follow cascade must not reinterpret the empty top-level
        // reference or rewrite a pool revision.
        if spec.template_pool.is_some() {
            diagnostic.outcome(Outcome::NotApplicable);
            return Ok(false);
        }
        // Shared-pool fleets (spec 0037 §5 stage 2): the spec carries only
        // the bare pool key; member pins live on pool revisions. Catch up
        // to the pool's current revision on the next pass, like a
        // single-template fleet. Member input validation already happened at pool
        // admission (re-resolved by the pool cascade), so there is no
        // input gate to re-run here.
        let target = FollowerTarget {
            key,
            head: &head,
            latest: &latest,
        };
        if let Some(pool_key) = spec
            .template_pool_ref
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return self
                .maybe_upgrade_pool_follower(&target, &spec, pool_key, now, &mut diagnostic)
                .await;
        }
        // ARD-0029: every live fleet follows its profile's Active revision;
        // legacy exact pins normalized on read above land here as followers.
        // Resolve the CURRENT Active revision directly: the retained-pin
        // shortcut in admission resolution would freeze the lagging pin.
        let pin = match self.resolve_template_ref(&spec.template_profile_ref).await {
            Ok(pin) => pin,
            Err(error) => {
                diagnostic.reason(
                    Code::RolloutNoActiveRevision,
                    StageId::DependencyResolution,
                    true,
                );
                // No Active revision (retired/unpublished profile) is a
                // routine skip, not an error.
                tracing::debug!(fleet = %key, summary = %error.summary, "follow resolution unavailable");
                return Ok(false);
            }
        };
        if let Some(d) = &mut diagnostic.0 {
            d.observation.question.rollout = Some(DiagnosticRollout {
                desired_revision: head.desired_revision.to_string(),
                observed_revision: head.observed_revision.to_string(),
                previous_pin: latest
                    .template_profile_key
                    .clone()
                    .zip(latest.template_revision)
                    .map(|(key, revision)| DiagnosticPin {
                        key,
                        revision: revision.to_string(),
                    }),
                candidate_pin: Some(DiagnosticPin {
                    key: pin.0.clone(),
                    revision: pin.1.to_string(),
                }),
            });
        }
        diagnostic.pass(StageId::DependencyResolution);
        let lagging = latest.template_profile_key.as_deref() != Some(pin.0.as_str())
            || latest.template_revision != Some(pin.1);
        if !lagging {
            diagnostic.outcome(Outcome::Satisfied);
            return Ok(false);
        }
        // Re-validate retained inputs against the NEW pin's finite alias
        // policy and artifact schema — the same gate a manual replacement
        // passes (spec 0002 §5.1); an incompatible follower skips instead
        // of silently changing its input contract.
        let materials = self
            .load_template_inputs(&pin, super::validation::InputContext::Fleet)
            .await?;
        if let Err(e) = materials.validate(&spec.template_inputs)? {
            diagnostic.reason(
                Code::RolloutInputsIncompatible,
                StageId::InputCompatibility,
                true,
            );
            tracing::warn!(fleet = %key, summary = %e.summary, "follow upgrade skipped: inputs incompatible with active revision");
            return Ok(false);
        }
        if let Err(error) = self
            .validate_template_backend(&pin.0, pin.1, &pin.2, &spec, &spec.template_inputs)
            .await
        {
            diagnostic.reason(
                Code::RolloutBackendIncompatible,
                StageId::InputCompatibility,
                true,
            );
            tracing::warn!(fleet = %key, summary = %error.summary, "follow upgrade skipped: incompatible runner backend");
            return Ok(false);
        }
        diagnostic.pass(StageId::InputCompatibility);
        // No occupancy gate (spec 0023 §3): live Generations keep their admitted
        // template pins and finish on them; the new revision is for new Creates.
        diagnostic.pass(StageId::Occupancy);
        let committed = self
            .commit_follow_replace(&target, Some(pin), None, now, &mut diagnostic)
            .await?;
        if committed {
            tracing::info!(fleet = %key, revision = head.desired_revision, "follower upgraded to active template revision");
        }
        Ok(committed)
    }
    /// Shared-pool fleet catch-up (spec 0037 §5 stage 2): re-mint the
    /// fleet revision onto the pool's CURRENT revision as soon as it lags,
    /// occupied or not. The spec is verbatim; only the frozen
    /// `(pool_key, pool_revision)` routing context moves.
    async fn maybe_upgrade_pool_follower(
        &self,
        target: &FollowerTarget<'_>,
        spec: &FleetSpec,
        pool_key: &str,
        now: i64,
        diagnostic: &mut Capture,
    ) -> CoreResult<bool> {
        let Some(pool_head) = self.store.template_pool_get(pool_key).await? else {
            diagnostic.reason(
                Code::RolloutNoActiveRevision,
                StageId::DependencyResolution,
                true,
            );
            return Ok(false);
        };
        // A deleted pool keeps routing the fleet's frozen revision — the
        // retention guarantee (spec 0037 §7); deletion is blocked while
        // referenced, so a tombstone here means the reference predates the
        // guard: never chase it.
        if pool_head.tombstone || pool_head.deletion_marker {
            return Ok(false);
        }
        let frozen = target
            .latest
            .template_pool_ref
            .clone()
            .unwrap_or_else(|| (pool_key.to_string(), 0));
        if let Some(d) = &mut diagnostic.0 {
            d.observation.question.rollout = Some(DiagnosticRollout {
                desired_revision: target.head.desired_revision.to_string(),
                observed_revision: target.head.observed_revision.to_string(),
                previous_pin: Some(DiagnosticPin {
                    key: frozen.0.clone(),
                    revision: frozen.1.to_string(),
                }),
                candidate_pin: Some(DiagnosticPin {
                    key: pool_key.into(),
                    revision: pool_head.desired_revision.to_string(),
                }),
            });
        }
        if frozen.1 >= pool_head.desired_revision {
            diagnostic.outcome(Outcome::Satisfied);
            return Ok(false);
        }
        // A shared pool is provider-neutral, but every member must match
        // this Fleet before its routing revision moves.
        let Some(pool) = self.store.template_pool_revision_latest(pool_key).await? else {
            return Ok(false);
        };
        if pool.revision != pool_head.desired_revision {
            return Ok(false);
        }
        if let Err(error) = self.validate_pool_backends(&pool.members, spec).await {
            diagnostic.reason(
                Code::RolloutBackendIncompatible,
                StageId::InputCompatibility,
                true,
            );
            tracing::warn!(fleet = %target.key, summary = %error.summary, "pool follow upgrade skipped: incompatible runner backend");
            return Ok(false);
        }
        // No occupancy gate (spec 0037 §5): live Generations keep the member
        // pins they were admitted under and retire on their own; only new
        // Generations draw from the current pool revision.
        diagnostic.pass(StageId::InputCompatibility);
        diagnostic.pass(StageId::Occupancy);
        let committed = self
            .commit_follow_replace(
                target,
                None,
                Some((pool_key.to_string(), pool_head.desired_revision)),
                now,
                diagnostic,
            )
            .await?;
        if committed {
            tracing::info!(fleet = %target.key, pool = %pool_key, pool_revision = pool_head.desired_revision, "pool follower upgraded to current pool revision");
        }
        Ok(committed)
    }

    async fn commit_follow_replace(
        &self,
        target: &FollowerTarget<'_>,
        template: Option<(String, i64, String, String)>,
        template_pool_ref: Option<shaula_core::template_pool::FleetPoolRef>,
        now: i64,
        diagnostic: &mut Capture,
    ) -> CoreResult<bool> {
        let revision = target.head.desired_revision + 1;
        let change_id = self.new_id();
        let change = ChangeView {
            id: change_id.clone(),
            resource_kind: "fleet".into(),
            resource_key: target.key.to_string(),
            revision,
            kind: "Replace".into(),
            state: "Pending".into(),
            reason: None,
        };
        let facts = MutationFacts {
            idempotency_operation: "v1:PUT",
            authentication: Default::default(),
            resource_kind: "fleet",
            resource_key: target.key.to_string(),
            incarnation: target.head.incarnation.clone(),
            revision,
            spec_json: target.latest.spec_json.clone(),
            template,
            template_pool: Vec::new(),
            template_pool_ref,
            auth_desired: Some(target.latest.auth_desired.clone()),
            inputs_digest: target.latest.inputs_digest.clone(),
            actor: "shaula-daemon".to_string(),
            now,
            change,
            outbox_topic: "fleet.change".to_string(),
            outbox_payload: format!("{{\"change\":\"{change_id}\"}}"),
            idempotency: None,
        };
        // Serialize against in-flight admission claims exactly like an operator PUT (R6-02).
        let effect_gate = self.effect_gates.acquire_exclusive(target.key).await;
        let committed = self.store.commit_fleet_mutation(facts).await;
        drop(effect_gate);
        match committed? {
            Ok(()) => Ok(true),
            Err(fence) => {
                diagnostic.reason(Code::RolloutCommitDeferred, StageId::CommitFence, true);
                tracing::debug!(fleet = %target.key, reason = %fence_summary(&fence), "follow upgrade not committed");
                Ok(false)
            }
        }
    }
}

fn fence_summary(error: &MutationError) -> String {
    format!("{error:?}")
}
