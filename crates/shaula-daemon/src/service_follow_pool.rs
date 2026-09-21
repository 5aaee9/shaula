//! Re-resolve shared TemplatePool pins before the Fleet follow pass.

use super::{fence_summary, ControlPlane, CoreError, CoreResult, ReasonCode};

impl ControlPlane {
    /// Cascade stage 1 (spec 0037 §5): every live pool re-resolves its
    /// members' bare `template_profile_ref` keys to the profiles' current
    /// Active revisions and mints ONE new pool revision when any pin
    /// moved — regardless of how many fleets reference the pool. A pool
    /// owns no runners, so there is no occupancy gate here; referencing
    /// fleets catch up in stage 2 on their own drain boundary.
    pub(super) async fn cascade_pool_follow_upgrades(&self, now: i64) -> CoreResult<u32> {
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
                    super::super::validate_inputs(&member.template_inputs, &policy, Some(&schema))
                {
                    tracing::warn!(pool = %key, member = %member.key, summary = %e.summary, "pool follow upgrade skipped: inputs incompatible with active revision");
                    return Ok(false);
                }
                lagging = true;
            }
            let inputs_digest = super::super::template_inputs_digest(&member.template_inputs)?;
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
        let inputs_digest = super::super::sha256_digest(latest.spec_json.as_bytes());
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
}
