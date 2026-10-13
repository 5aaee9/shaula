//! Forgejo Pool Retirement and Runner Maximum Lifetime: the shared selection in
//! [`crate::pool_driver`] over this backend's Runner Registrations.

use shaula_core::error::CoreResult;

use super::ForgejoPoolSupervisor;
use crate::pool_driver::Retirement;
use crate::runner_operation::forgejo::ForgejoRegistrations;
use crate::runner_operation::{DestroyTally, RunnerOperation};

impl ForgejoPoolSupervisor {
    fn retirement(&self) -> Retirement<'_, ForgejoRegistrations<'_>> {
        Retirement {
            operation: RunnerOperation {
                diagnostic_guard: Some(&self.guard),
                lifecycle: self.lifecycle.as_ref(),
                store: self.store.as_ref(),
                runtime: self.runtime.as_ref(),
                clock: Some(self.clock.as_ref()),
                artifact_root: &self.artifact_root,
                operation_timeout: self.operation_timeout,
                apply_intent: &self.apply_intent_sink,
                provider: shaula_core::fleet::FleetProviderKind::Forgejo,
                registrations: ForgejoRegistrations {
                    fleet_key: &self.fleet_key,
                    guard: &self.guard,
                    lifecycle: self.lifecycle.as_ref(),
                    store: self.store.as_ref(),
                    gates: &self.gates,
                    forgejo: self.forgejo.as_ref(),
                    runtime: self.runtime.as_ref(),
                    labels: &self.labels,
                },
            },
            fleet_key: &self.fleet_key,
            guard: &self.guard,
            lifecycle: self.lifecycle.as_ref(),
            store: self.store.as_ref(),
            destroy_limit: &self.destroy_limit,
        }
    }

    pub(super) async fn destroy_excess(&self, target: u64, now: i64) -> CoreResult<DestroyTally> {
        self.retirement().destroy_excess(target, now).await
    }

    pub(super) async fn expire_runners(&self, now: i64) -> CoreResult<DestroyTally> {
        self.retirement()
            .expire(now, self.runner_max_lifetime)
            .await
    }
}
