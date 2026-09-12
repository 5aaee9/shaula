//! Retirement and destroy flow, split to keep files within 400 lines
//! (AGENTS.md).

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::ports::{RemovalOutcome, TemplateDestroyRequest};
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use super::FleetSupervisor;

/// One bounded IaC engine operation completed with the given outcome.
fn record_iac(result: MetricResult) {
    TelemetryHandle::new().record(MetricOperation::IaC, result, 1);
}

impl FleetSupervisor {
    /// Retirement: GitHub removal gate (`JobStillRunning` authoritative)
    /// then delete-only destroy with the original artifact/state.
    ///
    /// Convergence (R9-10): `Retiring`, `DestroyPending` and `Destroying`
    /// generations are ALL re-selected every tick — a Destroy failure or
    /// a crash mid-destroy leaves durable state that the next tick
    /// resumes, instead of stranding the generation forever. The
    /// retirement intent is persisted (`Retiring`) BEFORE the remote
    /// runner removal, so a lost removal response still shows durable
    /// evidence.
    #[tracing::instrument(name = "shaula.iac.generation_destroy", skip_all, fields(fleet_key = %self.config.fleet_key))]
    pub(crate) async fn retire_excess(&self, mut excess: i64, now: i64) -> CoreResult<u32> {
        use shaula_core::lifecycle::GenerationState;
        let mut destroyed = 0u32;
        let generations = self
            .store
            .generations_for_fleet(&self.config.fleet_key)
            .await?;
        for generation in generations {
            // R10-03 follow-up: a Retiring generation whose removal was
            // BLOCKED (`JobStillRunning`) must be re-gated — the removal
            // is idempotent (Removed/AlreadyAbsent proceed, the safety
            // gate blocks again), so Retiring generations with a known
            // runner re-run the gate instead of skipping straight to the
            // destroy effect.
            let mut needs_removal = matches!(
                generation.state,
                GenerationState::Idle | GenerationState::Retiring
            ) && generation.github_runner_id.is_some();
            // Spec 0024 §2.1: a CleanupRequired generation past the
            // one-tick grace is driven through the SAME destroy chain
            // instead of being stranded for quarantine — its runner
            // entity (if any) is removed and its infrastructure
            // destroyed; only unprovable destroys quarantine (below).
            let cleanup_due = generation.state == GenerationState::CleanupRequired
                && now.saturating_sub(generation.updated_at) >= 60_000;
            let resumable_destroy = matches!(
                generation.state,
                GenerationState::Retiring
                    | GenerationState::DestroyPending
                    | GenerationState::Destroying
            );
            if !needs_removal && !resumable_destroy && !cleanup_due {
                continue;
            }
            if !resumable_destroy && !cleanup_due && excess <= 0 {
                continue;
            }
            let mut state = generation.state;
            let _permit =
                self.limits.destroy.acquire().await.map_err(|_| {
                    CoreError::new(ReasonCode::Internal, "destroy scheduler stopped")
                })?;
            if state == GenerationState::Idle {
                // Durable retirement intent FIRST: the removal is an
                // external effect, so the ledger must already show the
                // generation was being retired when it happened. The
                // LOCAL `state` mirror tracks every persisted advance so
                // the chain below never attempts an illegal transition
                // (R10-04: Idle -> Retiring -> DestroyPending ->
                // Destroying, never a skip).
                self.store
                    .generation_advance(&generation.id, GenerationState::Retiring, now)
                    .await?;
                state = GenerationState::Retiring;
                excess -= 1;
            }
            if state == GenerationState::CleanupRequired {
                // Same durable-intent rule as Idle: the cleanup path's
                // external effects begin with runner removal, so the
                // ledger must show Retiring first (legal transition).
                self.store
                    .generation_advance(&generation.id, GenerationState::Retiring, now)
                    .await?;
                state = GenerationState::Retiring;
                // The cleanup entry also gates on runner removal like a
                // fresh retirement (the entity usually self-deregistered;
                // AlreadyAbsent makes this idempotent).
                if generation.github_runner_id.is_some() {
                    needs_removal = true;
                }
            }
            if needs_removal {
                let Some(runner_id) = generation.github_runner_id else {
                    continue;
                };
                let Some(reference) = self.handoff.auth_generation_ref(&generation.id).await?
                else {
                    // Cleanup cannot safely remove a runner without the
                    // exact authority that admitted the generation. Keep
                    // the durable evidence, but quarantine instead of
                    // leaving the generation in Retiring forever.
                    tracing::error!(
                        generation = %generation.id,
                        "generation auth reference missing; quarantining cleanup"
                    );
                    self.store
                        .generation_advance(&generation.id, GenerationState::Quarantined, now)
                        .await?;
                    continue;
                };
                let current = (
                    self.config.auth_profile_key.clone(),
                    self.config.auth_revision,
                );
                let github = if reference == current {
                    Some(&self.github)
                } else {
                    self.revision_clients.get(&reference)
                };
                let Some(github) = github else {
                    tracing::error!(
                        generation = %generation.id,
                        auth_profile = %reference.0,
                        auth_revision = reference.1,
                        "generation auth client unavailable; quarantining cleanup"
                    );
                    self.store
                        .generation_advance(&generation.id, GenerationState::Quarantined, now)
                        .await?;
                    continue;
                };
                match github.remove_runner(runner_id).await {
                    Ok(RemovalOutcome::Removed) | Ok(RemovalOutcome::AlreadyAbsent) => {}
                    // Authoritative safety gate: keep the resource, retry
                    // next tick — the durable Retiring state makes this
                    // tick's attempt resumable instead of lost.
                    Ok(RemovalOutcome::JobStillRunning) => continue,
                    Err(_) => continue,
                }
            }
            {
                // Advance the durable chain to `Destroying` before the
                // effect — every step follows the persisted transition
                // chain (Retiring -> DestroyPending -> Destroying) using
                // the TRACKED state, never a skip from Idle (R10-04).
                if state == GenerationState::Retiring {
                    self.store
                        .generation_advance(&generation.id, GenerationState::DestroyPending, now)
                        .await?;
                    state = GenerationState::DestroyPending;
                }
                if state == GenerationState::DestroyPending {
                    self.store
                        .generation_advance(&generation.id, GenerationState::Destroying, now)
                        .await?;
                }
                let (_, stored_digest) = self
                    .handoff
                    .template_protected_bindings(
                        &generation.template_profile_key,
                        generation.template_revision,
                    )
                    .await?
                    .ok_or_else(|| {
                        CoreError::new(
                            ReasonCode::TemplateInvalid,
                            "admitted bindings missing for destroy",
                        )
                    })?;
                self.handoff
                    .ensure_artifact_cached(&generation.template_artifact_digest)
                    .await?;
                let artifact_dir = shaula_core::artifact_layout::artifact_dir(
                    &self.config.artifact_root,
                    &generation.template_artifact_digest,
                )
                .ok_or_else(|| {
                    CoreError::new(
                        ReasonCode::Internal,
                        "artifact digest malformed for destroy",
                    )
                })?;
                let manifest_path = artifact_dir.join("profile.yaml");
                let managed_shape = std::fs::read_to_string(&manifest_path)
                    .ok()
                    .and_then(|m| {
                        serde_yaml::from_str::<shaula_core::template::ProfileManifest>(&m).ok()
                    })
                    .filter(|m: &shaula_core::template::ProfileManifest| m.validate().is_ok())
                    .map(|m| m.managed_resource_shape)
                    .unwrap_or_default();
                // The durable ORIGINAL Create provenance: the destroy
                // must re-verify the SAME engine/input/artifact pins
                // (spec 0004 §5, exact provenance). A generation with
                // no Create record is corrupt-but-isolated: quarantine
                // it and keep retiring the remaining excess instead of
                // blocking the whole loop.
                let original_provenance = match self
                    .store
                    .operation_original_provenance(&generation.id)
                    .await?
                {
                    Some(provenance) => provenance,
                    None => {
                        tracing::error!(
                            generation = %generation.id,
                            "Create provenance missing for destroy; quarantining"
                        );
                        self.store
                            .generation_advance(
                                &generation.id,
                                shaula_core::lifecycle::GenerationState::Quarantined,
                                now,
                            )
                            .await?;
                        continue;
                    }
                };
                // The ORIGINAL post-Create state identity (F07): a
                // generation without a recorded identity has no
                // ownership proof — quarantine, never reconstruct.
                let Some((state_lineage, state_serial)) =
                    self.store.generation_state_identity(&generation.id).await?
                else {
                    tracing::error!(
                        generation = %generation.id,
                        "state identity missing for destroy; quarantining"
                    );
                    self.store
                        .generation_advance(
                            &generation.id,
                            shaula_core::lifecycle::GenerationState::Quarantined,
                            now,
                        )
                        .await?;
                    continue;
                };
                // A retry attempt may see a serial advanced by our
                // OWN prior partial destroy apply; the FIRST attempt
                // must match the original serial exactly.
                let allow_serial_advance = self
                    .store
                    .generation_destroy_attempted(&generation.id)
                    .await?;
                let apply_intent = crate::apply_intent::TrackedApplyIntentSink::new(
                    self.config.apply_intent_sink.clone(),
                );
                let request = TemplateDestroyRequest {
                    pinned_artifact_digest: generation.template_artifact_digest.clone(),
                    workspace_path: std::path::PathBuf::from(&generation.workspace_path),
                    artifact_dir,
                    expected_bindings_digest: shaula_core::template::BindingsDigest(stored_digest),
                    generation_id: generation.id.clone(),
                    managed_shape,
                    environment: Vec::new(),
                    timeout: self.config.operation_timeout,
                    original_provenance,
                    original_state: shaula_core::ports::OriginalStateIdentity {
                        lineage: state_lineage,
                        serial: state_serial,
                        allow_serial_advance,
                    },
                    apply_intent_sink: Some(apply_intent.clone()),
                };
                match self.runtime.destroy(request).await {
                    Ok(_) => {
                        record_iac(MetricResult::Ok);
                        apply_intent.complete(self.store.as_ref(), now).await?;
                        self.store
                            .generation_advance(
                                &generation.id,
                                shaula_core::lifecycle::GenerationState::Destroyed,
                                now,
                            )
                            .await?;
                        destroyed += 1;
                    }
                    Err(_) => {
                        record_iac(MetricResult::Failed);
                        self.store
                            .generation_advance(
                                &generation.id,
                                shaula_core::lifecycle::GenerationState::DestroyPending,
                                now,
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(destroyed)
    }
}
