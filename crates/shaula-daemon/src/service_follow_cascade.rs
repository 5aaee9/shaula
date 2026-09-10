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
}

fn fence_summary(error: &MutationError) -> String {
    format!("{error:?}")
}
