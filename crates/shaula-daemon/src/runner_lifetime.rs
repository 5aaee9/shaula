//! Shared hard-timeout destruction. Provider deregistration follows, never gates
//! termination of an overlong job. Ordinary idle retirement remains busy-safe.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetProviderKind;
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::ports::{
    ApplyIntentSink, Clock, OriginalStateIdentity, TemplateDestroyRequest, TemplateRuntimePort,
};
use shaula_core::registry::{ControlPlaneStore, GenerationRecord, LifecycleStore};
use shaula_core::template::{BindingsDigest, ProfileManifest};
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

pub(crate) struct RunnerReaper<'a> {
    pub lifecycle: &'a dyn LifecycleStore,
    pub store: &'a dyn ControlPlaneStore,
    pub runtime: &'a dyn TemplateRuntimePort,
    pub clock: Option<&'a dyn Clock>,
    pub artifact_root: &'a Path,
    pub operation_timeout: Duration,
    pub apply_intent: &'a Arc<dyn ApplyIntentSink>,
    pub provider: FleetProviderKind,
}

impl RunnerReaper<'_> {
    pub async fn due(
        &self,
        fleet: &str,
        now: i64,
        limit: Duration,
    ) -> CoreResult<Vec<GenerationRecord>> {
        let mut due = Vec::new();
        for generation in self.lifecycle.generations_for_fleet(fleet).await? {
            if matches!(
                generation.state,
                G::CreatePending | G::Quarantined | G::Destroyed
            ) {
                continue;
            }
            if self
                .lifecycle
                .generation_lifetime(&generation.id)
                .await?
                .is_due(now, limit)
            {
                due.push(generation);
            }
        }
        Ok(due)
    }

    /// True means resources are durably gone, NOT that remote registration is gone.
    pub async fn destroy_resources(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<bool> {
        let id = &generation.id;
        if !self.lifecycle.generation_request_expiry(id, now).await? {
            return Ok(false);
        }
        if self
            .lifecycle
            .generation_lifetime(id)
            .await?
            .resources_destroyed_at
            .is_some()
        {
            return Ok(true);
        }
        tracing::warn!(fleet = %generation.fleet_key, generation = %id,
            "runner maximum lifetime reached; terminating resources even if a job is running");
        // Hard timeout relaxes only the busy/idle gate, never resource ownership.
        let Some(original_provenance) = self.lifecycle.operation_original_provenance(id).await?
        else {
            return self
                .quarantine(id, now, "Create provenance missing for hard timeout")
                .await;
        };
        let Some((lineage, serial)) = self.lifecycle.generation_state_identity(id).await? else {
            return self
                .quarantine(id, now, "state identity missing for hard timeout")
                .await;
        };
        let Some((_, digest)) = self
            .store
            .template_protected_bindings(
                &generation.template_profile_key,
                generation.template_revision,
            )
            .await?
        else {
            return self
                .quarantine(id, now, "admitted bindings missing for hard timeout")
                .await;
        };
        self.store
            .ensure_artifact_cached(&generation.template_artifact_digest)
            .await?;
        let artifact_dir = shaula_core::artifact_layout::artifact_dir(
            self.artifact_root,
            &generation.template_artifact_digest,
        )
        .ok_or_else(|| {
            CoreError::new(
                ReasonCode::TemplateInvalid,
                "hard timeout artifact digest malformed",
            )
        })?;
        let manifest = std::fs::read_to_string(artifact_dir.join("profile.yaml"))
            .ok()
            .and_then(|text| serde_yaml::from_str::<ProfileManifest>(&text).ok());
        let Some(manifest) = manifest.filter(|m| m.validate_for_provider(self.provider).is_ok())
        else {
            return self
                .quarantine(id, now, "admitted manifest invalid for hard timeout")
                .await;
        };
        self.advance_to_destroying(generation, now).await?;
        let apply_intent =
            crate::apply_intent::TrackedApplyIntentSink::new(self.apply_intent.clone());
        let request = TemplateDestroyRequest {
            pinned_artifact_digest: generation.template_artifact_digest.clone(),
            workspace_path: generation.workspace_path.clone().into(),
            artifact_dir,
            expected_bindings_digest: BindingsDigest(digest),
            generation_id: id.clone(),
            managed_shape: manifest.managed_resource_shape,
            environment: Vec::new(),
            timeout: self.operation_timeout,
            original_provenance,
            original_state: OriginalStateIdentity {
                lineage,
                serial,
                allow_serial_advance: self.lifecycle.generation_destroy_attempted(id).await?,
            },
            apply_intent_sink: Some(apply_intent.clone()),
        };
        let result = self.runtime.destroy(request).await;
        let now = self.clock.map_or(now, Clock::now_unix_ms);
        match result {
            Ok(_) => {
                TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Ok, 1);
                apply_intent.complete(self.lifecycle, now).await?;
                self.lifecycle
                    .generation_resources_destroyed(id, now)
                    .await?;
                Ok(true)
            }
            Err(_) => {
                TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Failed, 1);
                tracing::warn!(generation = %id, "hard timeout resource destruction failed; will retry");
                self.lifecycle
                    .generation_advance(id, G::DestroyPending, now)
                    .await?;
                Ok(false)
            }
        }
    }

    pub async fn finish(&self, id: &str, now: i64) -> CoreResult<()> {
        self.lifecycle
            .generation_advance(id, G::Destroyed, self.clock.map_or(now, Clock::now_unix_ms))
            .await?;
        tracing::info!(generation = %id, "hard timeout cleanup complete: resources and registration removed");
        Ok(())
    }

    async fn quarantine(&self, id: &str, now: i64, message: &str) -> CoreResult<bool> {
        tracing::error!(generation = %id, "{message}; quarantining");
        self.lifecycle
            .generation_advance(id, G::Quarantined, now)
            .await?;
        Ok(false)
    }

    async fn advance_to_destroying(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<()> {
        let mut state = generation.state;
        loop {
            let next = match state {
                G::Creating | G::WaitingOnline => G::CleanupRequired,
                G::Idle | G::Busy | G::CleanupRequired => G::Retiring,
                G::Retiring => G::DestroyPending,
                G::DestroyPending => G::Destroying,
                G::Destroying => return Ok(()),
                _ => {
                    return Err(CoreError::new(
                        ReasonCode::OwnershipConflict,
                        "generation cannot expire",
                    ))
                }
            };
            self.lifecycle
                .generation_advance(&generation.id, next, now)
                .await?;
            state = next;
        }
    }
}
