//! Follow-latest template cascade (spec 0023): every live fleet follows
//! its profile's Active revision (follow-only since ARD-0029). The
//! level-triggered scan mints a replacement revision — unchanged spec,
//! new resolved pin — once the fleet drains to zero occupancy. Failures
//! degrade to per-fleet WARNs and retry next tick; the scan loop itself
//! never fails from one fleet's state.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetSpec;
use shaula_core::registry::{Actor, MutationError, Scope};

use super::ControlPlane;

impl ControlPlane {
    /// The cascade acts with the daemon's own service identity; its audit
    /// records stay distinguishable from operator mutations.
    fn daemon_actor() -> Actor {
        Actor {
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
        // up on their own occupancy boundary in the fleet pass below.
        upgraded += self.cascade_pool_follow_upgrades(now).await?;
        for (key, _revision, _phase) in self.store.fleet_list(&Self::daemon_actor()).await? {
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

    /// Cascade stage 1 (spec 0037 §5): every live pool re-resolves its
    /// members' bare `template_profile_ref` keys to the profiles' current
    /// Active revisions and mints ONE new pool revision when any pin
    /// moved — regardless of how many fleets reference the pool. A pool
    /// owns no runners, so there is no occupancy gate here; referencing
    /// fleets catch up in stage 2 on their own drain boundary.
    async fn cascade_pool_follow_upgrades(&self, now: i64) -> CoreResult<u32> {
        let mut upgraded = 0u32;
        for (key, _revision, _incarnation) in self.store.template_pool_list().await? {
            match self.maybe_upgrade_pool(&key, now).await {
                Ok(true) => upgraded += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(pool = %key, summary = %error.summary, "pool follow upgrade failed");
                }
            }
        }
        Ok(upgraded)
    }

    async fn maybe_upgrade_pool(&self, key: &str, now: i64) -> CoreResult<bool> {
        let Some(head) = self.store.template_pool_get(key).await? else {
            return Ok(false);
        };
        if head.tombstone || head.deletion_marker {
            return Ok(false);
        }
        let Some(latest) = self.store.template_pool_revision_latest(key).await? else {
            return Ok(false);
        };
        let spec: shaula_core::template_pool::TemplatePoolSpec =
            serde_json::from_str(&latest.spec_json)
                .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        // Re-resolve every member against current Active; one member whose
        // profile has no Active (retired/unpublished) skips the whole pool
        // this tick — a partial re-resolution would freeze mixed pins.
        let mut lagging = false;
        let mut members = Vec::with_capacity(spec.members.len());
        for member in &spec.members {
            let pin = match self
                .resolve_template_ref(&member.template_profile_ref)
                .await
            {
                Ok(pin) => pin,
                Err(error) => {
                    tracing::debug!(pool = %key, member = %member.key, summary = %error.summary, "pool member resolution unavailable");
                    return Ok(false);
                }
            };
            let previous = latest.members.iter().find(|m| m.key == member.key);
            let moved = previous.is_none_or(|p| {
                p.template_profile_key != pin.0
                    || p.template_revision != pin.1
                    || p.template_artifact_digest != pin.2
                    || p.template_attestation_id != pin.3
            });
            if moved {
                // Re-validate retained inputs against the NEW pin's alias
                // policy and artifact schema — the same gate a pool PUT
                // passes; an incompatible member skips the pool, never
                // silently changes its input contract.
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
                if let Err(e) =
                    super::validate_inputs(&member.template_inputs, &policy, Some(&schema))
                {
                    tracing::warn!(pool = %key, member = %member.key, summary = %e.summary, "pool follow upgrade skipped: inputs incompatible with active revision");
                    return Ok(false);
                }
                lagging = true;
            }
            let inputs_digest = super::template_inputs_digest(&member.template_inputs)?;
            members.push(shaula_core::template_pool::ResolvedTemplatePoolMember {
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
        if !lagging {
            return Ok(false);
        }
        let change_id = self.new_id();
        let revision = head.desired_revision + 1;
        let change = shaula_core::registry::ChangeView {
            id: change_id.clone(),
            resource_kind: "template_pool".into(),
            resource_key: key.to_string(),
            revision,
            kind: "Replace".into(),
            state: "Pending".into(),
            reason: None,
        };
        let inputs_digest = super::sha256_digest(latest.spec_json.as_bytes());
        let facts = shaula_core::registry::MutationFacts {
            resource_kind: "template_pool",
            resource_key: key.to_string(),
            incarnation: head.incarnation.clone(),
            revision,
            spec_json: latest.spec_json.clone(),
            template: None,
            template_pool: members,
            template_pool_ref: None,
            auth_desired: None,
            inputs_digest,
            actor: "shaula-daemon".to_string(),
            now,
            change,
            outbox_topic: "template_pool.change".to_string(),
            outbox_payload: format!("{{\"change\":\"{change_id}\"}}"),
            idempotency: None,
        };
        match self.store.commit_template_pool_mutation(facts).await? {
            Ok(()) => {
                tracing::info!(pool = %key, revision, "pool re-resolved to active template revisions");
                Ok(true)
            }
            // A lost pool head race defers to the next tick.
            Err(fence) => {
                tracing::debug!(pool = %key, reason = %fence_summary(&fence), "pool follow upgrade not committed");
                Ok(false)
            }
        }
    }

    async fn maybe_upgrade_follower(&self, key: &str, now: i64) -> CoreResult<bool> {
        let Some(head) = self.store.fleet_get(key).await? else {
            return Ok(false);
        };
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
            return Ok(false);
        }
        // Shared-pool fleets (spec 0037 §5 stage 2): the spec carries only
        // the bare pool key; member pins live on pool revisions. Catch up
        // to the pool's current revision on this fleet's zero-occupancy
        // boundary — the same deferred upgrade a single-template fleet
        // gets. Member input validation already happened at pool
        // admission (re-resolved by the pool cascade), so there is no
        // input gate to re-run here.
        if let Some(pool_key) = spec
            .template_pool_ref
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return self
                .maybe_upgrade_pool_follower(key, &head, &latest, pool_key, now)
                .await;
        }
        // ARD-0029: every live fleet follows its profile's Active revision;
        // legacy exact pins normalized on read above land here as followers.
        // Resolve the CURRENT Active revision directly: the retained-pin
        // shortcut in admission resolution would freeze the lagging pin.
        let pin = match self.resolve_template_ref(&spec.template_profile_ref).await {
            Ok(pin) => pin,
            Err(error) => {
                // No Active revision (retired/unpublished profile) is a
                // routine skip, not an error.
                tracing::debug!(fleet = %key, summary = %error.summary, "follow resolution unavailable");
                return Ok(false);
            }
        };
        let lagging = latest.template_profile_key.as_deref() != Some(pin.0.as_str())
            || latest.template_revision != Some(pin.1);
        if !lagging {
            return Ok(false);
        }
        // Re-validate retained inputs against the NEW pin's finite alias
        // policy and artifact schema — the same gate a manual replacement
        // passes (spec 0002 §5.1); an incompatible follower skips instead
        // of silently changing its input contract.
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
                "pinned artifact has a blank parameter schema document",
            ));
        }
        if let Err(e) = super::validate_inputs(&spec.template_inputs, &policy, Some(&schema)) {
            tracing::warn!(fleet = %key, summary = %e.summary, "follow upgrade skipped: inputs incompatible with active revision");
            return Ok(false);
        }
        // Cheap pre-check; the commit transaction re-checks occupancy under
        // the write lock (R9-04/R10-08), so a racing Create cannot slip in.
        if self.store.generations_occupancy(key).await? > 0 {
            tracing::debug!(fleet = %key, "follow upgrade deferred: fleet occupied");
            return Ok(false);
        }

        let draft = super::FleetMutationDraft {
            key: key.to_string(),
            incarnation: head.incarnation.clone(),
            revision: head.desired_revision + 1,
            // The spec is verbatim: the bare key already encodes "follow".
            spec_json: latest.spec_json.clone(),
            template: Some(pin),
            template_pool: Vec::new(),
            template_pool_ref: None,
            // Auth desired is untouched by a template upgrade; the auth
            // handoff retarget on rotation is a separate cascade (spec
            // 0023 §4) and the commit re-resolves it in-tx anyway (R9-03).
            auth_desired: Some(latest.auth_desired.clone()),
            inputs_digest: latest.inputs_digest.clone(),
            actor: "shaula-daemon".to_string(),
            kind: "Replace",
            now,
        };
        let change_id = self.new_id();
        let change = draft.change_view(&change_id);
        // Serialize against in-flight admission claims exactly like an
        // operator PUT (R6-02).
        let effect_gate = self.effect_gates.acquire_exclusive(key).await;
        let committed = self
            .store
            .commit_fleet_mutation(draft.into_facts(change, None))
            .await;
        drop(effect_gate);
        match committed? {
            Ok(()) => {
                tracing::info!(fleet = %key, revision = head.desired_revision, "follower upgraded to active template revision");
                Ok(true)
            }
            // A lost fence race or a just-created generation defers to the
            // next tick rather than failing the pass.
            Err(fence) => {
                tracing::debug!(fleet = %key, reason = %fence_summary(&fence), "follow upgrade not committed");
                Ok(false)
            }
        }
    }
    /// Shared-pool fleet catch-up (spec 0037 §5 stage 2): re-mint the
    /// fleet revision onto the pool's CURRENT revision once this fleet
    /// drains to zero occupancy. The spec is verbatim; only the frozen
    /// `(pool_key, pool_revision)` routing context moves.
    async fn maybe_upgrade_pool_follower(
        &self,
        key: &str,
        head: &shaula_core::registry::FleetHead,
        latest: &shaula_core::registry::store_port::FleetRevisionRow,
        pool_key: &str,
        now: i64,
    ) -> CoreResult<bool> {
        let Some(pool_head) = self.store.template_pool_get(pool_key).await? else {
            return Ok(false);
        };
        // A deleted pool keeps routing the fleet's frozen revision — the
        // retention guarantee (spec 0037 §7); deletion is blocked while
        // referenced, so a tombstone here means the reference predates the
        // guard: never chase it.
        if pool_head.tombstone || pool_head.deletion_marker {
            return Ok(false);
        }
        let frozen = latest
            .template_pool_ref
            .clone()
            .unwrap_or_else(|| (pool_key.to_string(), 0));
        if frozen.1 >= pool_head.desired_revision {
            return Ok(false);
        }
        // Cheap pre-check; the commit transaction re-checks occupancy
        // under the write lock (R9-04/R10-08).
        if self.store.generations_occupancy(key).await? > 0 {
            tracing::debug!(fleet = %key, "pool follow upgrade deferred: fleet occupied");
            return Ok(false);
        }
        let draft = super::FleetMutationDraft {
            key: key.to_string(),
            incarnation: head.incarnation.clone(),
            revision: head.desired_revision + 1,
            spec_json: latest.spec_json.clone(),
            template: None,
            template_pool: Vec::new(),
            template_pool_ref: Some((pool_key.to_string(), pool_head.desired_revision)),
            auth_desired: Some(latest.auth_desired.clone()),
            inputs_digest: latest.inputs_digest.clone(),
            actor: "shaula-daemon".to_string(),
            kind: "Replace",
            now,
        };
        let change_id = self.new_id();
        let change = draft.change_view(&change_id);
        let effect_gate = self.effect_gates.acquire_exclusive(key).await;
        let committed = self
            .store
            .commit_fleet_mutation(draft.into_facts(change, None))
            .await;
        drop(effect_gate);
        match committed? {
            Ok(()) => {
                tracing::info!(fleet = %key, pool = %pool_key, pool_revision = pool_head.desired_revision, "pool follower upgraded to current pool revision");
                Ok(true)
            }
            Err(fence) => {
                tracing::debug!(fleet = %key, reason = %fence_summary(&fence), "pool follow upgrade not committed");
                Ok(false)
            }
        }
    }
}

fn fence_summary(error: &MutationError) -> String {
    format!("{error:?}")
}
