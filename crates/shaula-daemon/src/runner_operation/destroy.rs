//! Destroy for Retirement: the durable Retirement intent first, then busy-safe
//! Runner Registration removal, then the Runner Resource. The Runner Resource
//! destroy re-verifies the original Create pins (spec 0004 §5); a Generation
//! that never started a Create apply has no Runner Resource (ARD-0040).

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::lifecycle::GenerationState as G;
use shaula_core::ports::{OriginalStateIdentity, PlanProvenance, TemplateDestroyRequest};
use shaula_core::registry::GenerationRecord;
use shaula_core::template::{BindingsDigest, ManagedResourceRole, ProfileManifest};
use shaula_observability::{MetricOperation, MetricResult, TelemetryHandle};

use super::{DestroyOutcome, Registration, RemovalGate, RunnerOperation, RunnerRegistrations};

/// Proof material a Runner Resource destroy re-verifies.
pub(super) struct DestroyProof {
    original_provenance: PlanProvenance,
    lineage: String,
    serial: u64,
    bindings_digest: String,
    artifact_dir: std::path::PathBuf,
    managed_shape: Vec<ManagedResourceRole>,
}

/// Outcome of one Runner Resource destroy effect, with its completion time.
pub(super) enum ResourceDestroy {
    Destroyed { at: i64 },
    Failed { at: i64 },
}

impl<R: RunnerRegistrations> RunnerOperation<'_, R> {
    /// One Retirement step for a selected Generation.
    #[tracing::instrument(name = "shaula.iac.generation_destroy", skip_all, fields(generation_id = %generation.id))]
    pub(crate) async fn retire(
        &self,
        generation: &GenerationRecord,
        now: i64,
    ) -> CoreResult<DestroyOutcome> {
        let id = generation.id.as_str();
        let lifetime = self.lifecycle.generation_lifetime(id).await?;
        if lifetime.expiry_requested_at.is_some() {
            // The resource-first Runner Maximum Lifetime path owns both checkpoints.
            return Ok(DestroyOutcome::Pending);
        }
        let create_started = self.create_started(id).await?;
        let mut state = generation.state;
        let rank = chain_rank(state).ok_or_else(|| cannot_destroy(state))?;
        // Past `Retiring` the chain has already proven the registration gone.
        if rank <= RETIRING_RANK {
            match self.admit_registration(generation, create_started).await? {
                Registration::Removable => {
                    // Durable Retirement intent BEFORE the removal effect (R10-04).
                    state = self.walk(id, state, G::Retiring, now).await?;
                    let gate = RemovalGate::Retirement { create_started };
                    match self.registrations.remove(generation, gate).await? {
                        Registration::Gone => {}
                        Registration::Busy | Registration::Unavailable => {
                            return Ok(DestroyOutcome::Pending);
                        }
                        Registration::Removable
                        | Registration::Unprovable
                        | Registration::Unrecorded => {
                            return self.quarantine(id, now, UNPROVEN_REMOVAL).await;
                        }
                    }
                }
                Registration::Gone => {
                    state = self.walk(id, state, G::Retiring, now).await?;
                }
                Registration::Busy | Registration::Unavailable => {
                    return Ok(DestroyOutcome::Pending);
                }
                Registration::Unprovable | Registration::Unrecorded => {
                    return self.quarantine(id, now, UNPROVEN_REMOVAL).await;
                }
            }
        }
        self.walk(id, state, G::Destroying, now).await?;
        if !create_started {
            self.lifecycle
                .generation_advance(id, G::Destroyed, now)
                .await?;
            return Ok(DestroyOutcome::Destroyed);
        }
        let proof = match self.destroy_proof(generation).await? {
            Ok(proof) => proof,
            Err(reason) => return self.quarantine(id, now, reason).await,
        };
        match self.destroy_resources(generation, proof, now).await? {
            ResourceDestroy::Destroyed { at } => {
                self.lifecycle
                    .generation_advance(id, G::Destroyed, at)
                    .await?;
                Ok(DestroyOutcome::Destroyed)
            }
            ResourceDestroy::Failed { at } => {
                self.lifecycle
                    .generation_advance(id, G::DestroyPending, at)
                    .await?;
                Ok(DestroyOutcome::Pending)
            }
        }
    }

    async fn admit_registration(
        &self,
        generation: &GenerationRecord,
        create_started: bool,
    ) -> CoreResult<Registration> {
        match self
            .registrations
            .admit_removal(generation, create_started)
            .await?
        {
            Registration::Unrecorded if !create_started => {
                self.registrations.prove_unrecorded_absent(generation).await
            }
            // A started Create always retains its registration identity.
            Registration::Unrecorded => Ok(Registration::Unprovable),
            verdict => Ok(verdict),
        }
    }

    /// Advances along the Destroy chain until `until` is reached; states
    /// already at or past it are left unchanged.
    pub(super) async fn walk(&self, id: &str, from: G, until: G, now: i64) -> CoreResult<G> {
        let target = chain_rank(until).ok_or_else(|| cannot_destroy(until))?;
        let mut state = from;
        loop {
            let rank = chain_rank(state).ok_or_else(|| cannot_destroy(state))?;
            if rank >= target {
                return Ok(state);
            }
            let next = chain_next(state);
            self.lifecycle.generation_advance(id, next, now).await?;
            state = next;
        }
    }

    /// Loads the original Create pins. `Err(reason)` means the proof is
    /// definitively missing; a failed artifact cache restore is transient.
    pub(super) async fn destroy_proof(
        &self,
        generation: &GenerationRecord,
    ) -> CoreResult<Result<DestroyProof, &'static str>> {
        let id = generation.id.as_str();
        let Some(original_provenance) = self.lifecycle.operation_original_provenance(id).await?
        else {
            return Ok(Err("Create provenance missing for destroy"));
        };
        let Some((lineage, serial)) = self.lifecycle.generation_state_identity(id).await? else {
            return Ok(Err("state identity missing for destroy"));
        };
        let Some((_, bindings_digest)) = self
            .store
            .template_protected_bindings(
                &generation.template_profile_key,
                generation.template_revision,
            )
            .await?
        else {
            return Ok(Err("admitted bindings missing for destroy"));
        };
        self.store
            .ensure_artifact_cached(&generation.template_artifact_digest)
            .await?;
        let Some(artifact_dir) = shaula_core::artifact_layout::artifact_dir(
            self.artifact_root,
            &generation.template_artifact_digest,
        ) else {
            return Ok(Err("artifact digest malformed for destroy"));
        };
        let manifest = tokio::fs::read_to_string(artifact_dir.join("profile.yaml"))
            .await
            .ok()
            .and_then(|text| serde_yaml::from_str::<ProfileManifest>(&text).ok())
            .filter(|manifest| manifest.validate_for_provider(self.provider).is_ok());
        let Some(manifest) = manifest else {
            return Ok(Err("admitted manifest invalid for destroy"));
        };
        Ok(Ok(DestroyProof {
            original_provenance,
            lineage,
            serial,
            bindings_digest,
            artifact_dir,
            managed_shape: manifest.managed_resource_shape,
        }))
    }

    /// Runs the Runner Resource destroy against the proven pins. The caller
    /// has already advanced the Generation to `Destroying`.
    pub(super) async fn destroy_resources(
        &self,
        generation: &GenerationRecord,
        proof: DestroyProof,
        now: i64,
    ) -> CoreResult<ResourceDestroy> {
        let id = generation.id.as_str();
        let apply_intent =
            crate::apply_intent::TrackedApplyIntentSink::new(self.apply_intent.clone());
        let request = TemplateDestroyRequest {
            pinned_artifact_digest: generation.template_artifact_digest.clone(),
            workspace_path: std::path::PathBuf::from(&generation.workspace_path),
            artifact_dir: proof.artifact_dir,
            expected_bindings_digest: BindingsDigest(proof.bindings_digest),
            generation_id: generation.id.clone(),
            managed_shape: proof.managed_shape,
            environment: Vec::new(),
            timeout: self.operation_timeout,
            original_provenance: proof.original_provenance,
            original_state: OriginalStateIdentity {
                lineage: proof.lineage,
                serial: proof.serial,
                // A retry may observe a serial advanced by our own partial destroy.
                allow_serial_advance: self.lifecycle.generation_destroy_attempted(id).await?,
            },
            apply_intent_sink: Some(apply_intent.clone()),
        };
        let result = self.runtime.destroy(request).await;
        let at = self.now_after(now);
        match result {
            Ok(_) => {
                record_iac(MetricResult::Ok);
                apply_intent.complete(self.lifecycle, at).await?;
                Ok(ResourceDestroy::Destroyed { at })
            }
            Err(_) => {
                record_iac(MetricResult::Failed);
                tracing::warn!(generation = %id, "runner resource destroy failed; will retry");
                Ok(ResourceDestroy::Failed { at })
            }
        }
    }
}

const RETIRING_RANK: u8 = 2;
const UNPROVEN_REMOVAL: &str = "runner registration cannot be proven removed";

/// Position on the Destroy chain; `None` means the state cannot be destroyed.
fn chain_rank(state: G) -> Option<u8> {
    match state {
        G::Creating | G::WaitingOnline => Some(0),
        G::Idle | G::Busy | G::CleanupRequired => Some(1),
        G::Retiring => Some(RETIRING_RANK),
        G::DestroyPending => Some(3),
        G::Destroying => Some(4),
        G::CreatePending | G::Quarantined | G::Destroyed => None,
    }
}

fn chain_next(state: G) -> G {
    match state {
        G::Creating | G::WaitingOnline => G::CleanupRequired,
        G::Idle | G::Busy | G::CleanupRequired => G::Retiring,
        G::Retiring => G::DestroyPending,
        _ => G::Destroying,
    }
}

fn cannot_destroy(state: G) -> CoreError {
    CoreError::new(
        ReasonCode::OwnershipConflict,
        format!(
            "generation cannot be destroyed from {}",
            state.as_str_repr()
        ),
    )
}

fn record_iac(result: MetricResult) {
    TelemetryHandle::new().record(MetricOperation::IaC, result, 1);
}
