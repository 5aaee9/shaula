//! Registration removal and resource destruction require independent proof.

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState;
use shaula_core::ports::forgejo::{ForgejoRemovalOutcome, ForgejoTemplateEvidence};
use shaula_core::ports::{OriginalStateIdentity, TemplateDestroyRequest};
use shaula_core::registry::GenerationRecord;
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use super::{generation::read_manifest, readiness::observe_identity, ForgejoPoolSupervisor};

const IDLE_DEADLINE_MS: i64 = 600_000;

impl ForgejoPoolSupervisor {
    pub(super) async fn destroy_excess(&self, target: u64, now: i64) -> CoreResult<u32> {
        let generations = self
            .lifecycle
            .generations_for_fleet(&self.fleet_key)
            .await?;
        let effective = generations
            .iter()
            .filter(|g| g.state.counts_effective())
            .count() as u64;
        let mut idle_removals = effective.saturating_sub(target);
        let mut destroyed = 0_u32;
        for generation in generations {
            let resumable = matches!(
                generation.state,
                GenerationState::CleanupRequired
                    | GenerationState::Retiring
                    | GenerationState::DestroyPending
                    | GenerationState::Destroying
            );
            let idle = generation.state == GenerationState::Idle
                && (idle_removals > 0
                    || generation.fleet_revision != self.guard.desired_revision
                    || now.saturating_sub(generation.updated_at) >= IDLE_DEADLINE_MS);
            let retiring_revision = generation.state == GenerationState::WaitingOnline
                && generation.fleet_revision != self.guard.desired_revision;
            if !resumable && !idle && !retiring_revision {
                continue;
            }
            let _permit =
                self.destroy_limit.acquire().await.map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "destroy scheduler stopped")
                })?;
            if self.destroy_one(&generation, now).await? {
                destroyed = destroyed.saturating_add(1);
                idle_removals = idle_removals.saturating_sub(u64::from(idle));
            }
        }
        Ok(destroyed)
    }

    async fn destroy_one(&self, generation: &GenerationRecord, now: i64) -> CoreResult<bool> {
        let original = self
            .lifecycle
            .operation_original_provenance(&generation.id)
            .await?;
        let open = self
            .lifecycle
            .operations_open_for_generation(&generation.id)
            .await?;
        // No Create apply intent means no process could have received the token.
        let never_started = original.is_none() && !open.iter().any(|op| op.kind == "Create");
        let identity = self
            .lifecycle
            .generation_forgejo_runner(&generation.id)
            .await?;
        if identity.is_none()
            && (!never_started || open.iter().any(|op| op.kind == "ForgejoRegistration"))
        {
            quarantine(self, &generation.id, now).await?;
            return Ok(false);
        }
        {
            let _claim = self.gates.acquire_claim(&self.fleet_key).await;
            if self.current_head().await?.is_none() {
                return Ok(false);
            }
            // Re-read at the destructive boundary, not from the tick's old
            // snapshot (especially not an inventory taken before a Create).
            let Ok(runners) = self.forgejo.list_runners().await else {
                return Ok(false);
            };
            if let Some(identity) = &identity {
                if observe_identity(generation, identity, &runners).is_err() {
                    quarantine(self, &generation.id, now).await?;
                    return Ok(false);
                }
                let Ok(id) = u64::try_from(identity.0) else {
                    quarantine(self, &generation.id, now).await?;
                    return Ok(false);
                };
                let detail = match self.forgejo.get_runner(id).await {
                    Ok(detail) => detail,
                    Err(_) => return Ok(false),
                };
                let remote = match observe_identity(generation, identity, detail.as_slice()) {
                    Ok(remote) => remote,
                    Err(()) => {
                        quarantine(self, &generation.id, now).await?;
                        return Ok(false);
                    }
                };
                if let Some(remote) = remote {
                    let safe = if never_started {
                        // Positive, retained registration identity plus proof
                        // that no bootstrap was authorized. Labels may still
                        // be empty before the runner's first Declare.
                        matches!(remote.status.as_str(), "idle" | "offline")
                    } else {
                        remote.is_idle()
                            && super::is_owned_runner(remote, &self.labels)
                            && self.runtime.forgejo_template_evidence(&generation.id).await
                                == ForgejoTemplateEvidence::ProcessIdle
                    };
                    if !safe {
                        return Ok(false);
                    }
                    match self.forgejo.delete_runner(remote.id).await {
                        Ok(
                            ForgejoRemovalOutcome::Removed | ForgejoRemovalOutcome::AlreadyAbsent,
                        ) => {}
                        _ => return Ok(false),
                    }
                }
            } else if runners
                .iter()
                .any(|runner| runner.name == generation.runner_name)
            {
                quarantine(self, &generation.id, now).await?;
                return Ok(false);
            }
        }
        let mut state = generation.state;
        if state == GenerationState::WaitingOnline {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::CleanupRequired, now)
                .await?;
            state = GenerationState::CleanupRequired;
        }
        if matches!(
            state,
            GenerationState::Idle | GenerationState::Busy | GenerationState::CleanupRequired
        ) {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::Retiring, now)
                .await?;
            state = GenerationState::Retiring;
        }
        if state == GenerationState::Retiring {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::DestroyPending, now)
                .await?;
            state = GenerationState::DestroyPending;
        }
        if state == GenerationState::DestroyPending {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::Destroying, now)
                .await?;
        }
        if never_started {
            self.lifecycle
                .generation_advance(&generation.id, GenerationState::Destroyed, now)
                .await?;
            return Ok(true);
        }
        let Some(original_provenance) = original else {
            quarantine(self, &generation.id, now).await?;
            return Ok(false);
        };
        let Some((lineage, serial)) = self
            .lifecycle
            .generation_state_identity(&generation.id)
            .await?
        else {
            // Partial apply without original state proof is operator recovery,
            // never a guessed/reconstructed destroy or a second Create.
            quarantine(self, &generation.id, now).await?;
            return Ok(false);
        };
        let Some((_, stored_digest)) = self
            .store
            .template_protected_bindings(
                &generation.template_profile_key,
                generation.template_revision,
            )
            .await?
        else {
            quarantine(self, &generation.id, now).await?;
            return Ok(false);
        };
        self.store
            .ensure_artifact_cached(&generation.template_artifact_digest)
            .await?;
        let artifact_dir = shaula_core::artifact_layout::artifact_dir(
            &self.artifact_root,
            &generation.template_artifact_digest,
        )
        .ok_or_else(|| CoreError::new(ReasonCode::Internal, "Forgejo artifact digest malformed"))?;
        let manifest = read_manifest(&artifact_dir)?;
        manifest.validate_for_provider(shaula_core::fleet::FleetProviderKind::Forgejo)?;
        let apply_intent =
            crate::apply_intent::TrackedApplyIntentSink::new(self.apply_intent_sink.clone());
        let request = TemplateDestroyRequest {
            pinned_artifact_digest: generation.template_artifact_digest.clone(),
            workspace_path: std::path::PathBuf::from(&generation.workspace_path),
            artifact_dir,
            expected_bindings_digest: shaula_core::template::BindingsDigest(stored_digest),
            generation_id: generation.id.clone(),
            managed_shape: manifest.managed_resource_shape,
            environment: Vec::new(),
            timeout: self.operation_timeout,
            original_provenance,
            original_state: OriginalStateIdentity {
                lineage,
                serial,
                allow_serial_advance: self
                    .lifecycle
                    .generation_destroy_attempted(&generation.id)
                    .await?,
            },
            apply_intent_sink: Some(apply_intent.clone()),
        };
        match self.runtime.destroy(request).await {
            Ok(_) => {
                let now = self.clock.now_unix_ms();
                TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Ok, 1);
                apply_intent.complete(self.lifecycle.as_ref(), now).await?;
                self.lifecycle
                    .generation_advance(&generation.id, GenerationState::Destroyed, now)
                    .await?;
                Ok(true)
            }
            Err(_) => {
                TelemetryHandle::new().record(MetricOperation::IaC, MetricResult::Failed, 1);
                self.lifecycle
                    .generation_advance(
                        &generation.id,
                        GenerationState::DestroyPending,
                        self.clock.now_unix_ms(),
                    )
                    .await?;
                Ok(false)
            }
        }
    }
}

async fn quarantine(supervisor: &ForgejoPoolSupervisor, id: &str, now: i64) -> CoreResult<()> {
    supervisor
        .lifecycle
        .generation_advance(id, GenerationState::Quarantined, now)
        .await
}
