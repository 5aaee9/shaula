//! Forgejo Pool adapter for the Runner Backend seam. Forgejo `DELETE` has no
//! busy protection, so removal needs the exact ID + UUID + name + ephemeral
//! identity plus local process-idle evidence (spec 0026).

use shaula_core::error::CoreResult;
use shaula_core::ports::forgejo::{
    ForgejoPoolPort, ForgejoRemovalOutcome, ForgejoRunnerRef, ForgejoTemplateEvidence,
};
use shaula_core::ports::TemplateRuntimePort;
use shaula_core::registry::{
    ControlPlaneStore, FleetRuntimeGuard, GenerationRecord, LifecycleStore,
};

use super::{Registration, RemovalGate, RunnerRegistrations};
use crate::effect_gate::FleetEffectGates;

pub(crate) struct ForgejoRegistrations<'a> {
    pub fleet_key: &'a str,
    pub guard: &'a FleetRuntimeGuard,
    pub lifecycle: &'a dyn LifecycleStore,
    pub store: &'a dyn ControlPlaneStore,
    pub gates: &'a FleetEffectGates,
    pub forgejo: &'a dyn ForgejoPoolPort,
    pub runtime: &'a dyn TemplateRuntimePort,
    /// Normalized label names the Fleet's runners must carry.
    pub labels: &'a [String],
}

impl ForgejoRegistrations<'_> {
    /// Removal authority exists only for the exact admitted Fleet authority.
    async fn authority_current(&self) -> CoreResult<bool> {
        Ok(self
            .store
            .fleet_get(self.fleet_key)
            .await?
            .is_some_and(|head| !head.tombstone && FleetRuntimeGuard::from(&head) == *self.guard))
    }
}

/// The exact remote runner behind a recorded identity, observed under the
/// Fleet claim, or the verdict that ends the question.
enum Observed {
    Runner(ForgejoRunnerRef),
    Verdict(Registration),
}

impl ForgejoRegistrations<'_> {
    /// Observes the recorded runner by exact ID + UUID + name + ephemeral.
    /// `full_inventory` also rejects contradicting inventory entries.
    async fn observe(
        &self,
        generation: &GenerationRecord,
        full_inventory: bool,
    ) -> CoreResult<Observed> {
        let Some(identity) = self
            .lifecycle
            .generation_forgejo_runner(&generation.id)
            .await?
        else {
            return Ok(Observed::Verdict(Registration::Unrecorded));
        };
        if !self.authority_current().await? {
            return Ok(Observed::Verdict(Registration::Unavailable));
        }
        let Ok(id) = u64::try_from(identity.0) else {
            return Ok(Observed::Verdict(Registration::Unprovable));
        };
        if full_inventory {
            // Re-read at the decision boundary, never a pre-Create snapshot.
            let Ok(runners) = self.forgejo.list_runners().await else {
                return Ok(Observed::Verdict(Registration::Unavailable));
            };
            if observe_identity(generation, &identity, &runners).is_err() {
                return Ok(Observed::Verdict(Registration::Unprovable));
            }
        }
        let Ok(detail) = self.forgejo.get_runner(id).await else {
            return Ok(Observed::Verdict(Registration::Unavailable));
        };
        Ok(
            match observe_identity(generation, &identity, detail.as_slice()) {
                Ok(Some(remote)) => Observed::Runner(remote.clone()),
                Ok(None) if detail.is_none() => Observed::Verdict(Registration::Gone),
                Ok(None) | Err(()) => Observed::Verdict(Registration::Unprovable),
            },
        )
    }
}

impl RunnerRegistrations for ForgejoRegistrations<'_> {
    /// Forgejo `DELETE` has no busy protection: idleness is proven read-only
    /// before Retirement is recorded, so a refused runner keeps serving.
    async fn admit_removal(
        &self,
        generation: &GenerationRecord,
        create_started: bool,
    ) -> CoreResult<Registration> {
        let _claim = self.gates.acquire_claim(self.fleet_key).await;
        let remote = match self.observe(generation, true).await? {
            Observed::Runner(remote) => remote,
            Observed::Verdict(verdict) => return Ok(verdict),
        };
        let safe = if create_started {
            remote.is_idle()
                && is_owned_runner(&remote, self.labels)
                && self.runtime.forgejo_template_evidence(&generation.id).await
                    == ForgejoTemplateEvidence::ProcessIdle
        } else {
            // Retained identity plus proof that no bootstrap was authorized;
            // labels may still be empty before the runner's first Declare.
            matches!(remote.status.as_str(), "idle" | "offline")
        };
        Ok(if safe {
            Registration::Removable
        } else {
            Registration::Busy
        })
    }

    async fn remove(
        &self,
        generation: &GenerationRecord,
        _gate: RemovalGate,
    ) -> CoreResult<Registration> {
        let _claim = self.gates.acquire_claim(self.fleet_key).await;
        let remote = match self.observe(generation, false).await? {
            Observed::Runner(remote) => remote,
            Observed::Verdict(verdict) => return Ok(verdict),
        };
        Ok(match self.forgejo.delete_runner(remote.id).await {
            Ok(ForgejoRemovalOutcome::Removed | ForgejoRemovalOutcome::AlreadyAbsent) => {
                Registration::Gone
            }
            _ => Registration::Unavailable,
        })
    }

    async fn prove_unrecorded_absent(
        &self,
        generation: &GenerationRecord,
    ) -> CoreResult<Registration> {
        // An unresolved registration intent may have landed without an identity;
        // paginated inventory alone cannot prove it absent.
        if self
            .lifecycle
            .operations_open_for_generation(&generation.id)
            .await?
            .iter()
            .any(|operation| operation.kind == "ForgejoRegistration")
        {
            return Ok(Registration::Unprovable);
        }
        let _claim = self.gates.acquire_claim(self.fleet_key).await;
        if !self.authority_current().await? {
            return Ok(Registration::Unavailable);
        }
        let Ok(runners) = self.forgejo.list_runners().await else {
            return Ok(Registration::Unavailable);
        };
        Ok(
            if runners
                .iter()
                .any(|runner| runner.name == generation.runner_name)
            {
                Registration::Unprovable
            } else {
                Registration::Gone
            },
        )
    }
}

/// An ID match with changed UUID/name is NOT absence. A colliding name/UUID is
/// also contradictory evidence, even when the expected ID is missing.
pub(crate) fn observe_identity<'a>(
    generation: &GenerationRecord,
    identity: &(i64, String),
    runners: &'a [ForgejoRunnerRef],
) -> Result<Option<&'a ForgejoRunnerRef>, ()> {
    let mut observed = None;
    for runner in runners {
        if i64::try_from(runner.id).ok() == Some(identity.0)
            || runner.uuid == identity.1
            || runner.name == generation.runner_name
        {
            if observed.is_some()
                || i64::try_from(runner.id).ok() != Some(identity.0)
                || runner.uuid != identity.1
                || runner.name != generation.runner_name
                || !runner.ephemeral
            {
                return Err(());
            }
            observed = Some(runner);
        }
    }
    Ok(observed)
}

pub(crate) fn is_owned_runner(runner: &ForgejoRunnerRef, labels: &[String]) -> bool {
    runner.ephemeral
        && runner.is_known()
        && labels.iter().all(|label| runner.labels.contains(label))
}
