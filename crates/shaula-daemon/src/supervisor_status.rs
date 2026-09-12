use super::{FleetSupervisor, ReconcileReport};
use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::registry::{FleetObservation, FleetObservationPhase, FleetRuntimeGuard};
use std::sync::Arc;

impl FleetSupervisor {
    #[must_use]
    pub fn with_runtime_guard(mut self, guard: FleetRuntimeGuard) -> Self {
        self.runtime_guard = Some(guard);
        self
    }

    #[must_use]
    pub fn with_listener(mut self, listener: Option<Arc<crate::listener::FleetListener>>) -> Self {
        self.listener = listener;
        self
    }

    pub fn listener(&self) -> Option<Arc<crate::listener::FleetListener>> {
        self.listener.clone()
    }

    pub(super) async fn observe_reconcile(
        &self,
        result: &CoreResult<ReconcileReport>,
        captured_epoch: Option<i64>,
        now: i64,
    ) -> CoreResult<()> {
        let Some(guard) = &self.runtime_guard else {
            return Ok(());
        };
        let (phase, reason) = match result {
            Ok(report) if report.blocked => (
                FleetObservationPhase::Degraded,
                Some(report.reason.unwrap_or(ReasonCode::OwnershipProofFailed)),
            ),
            // A deletion-marked fleet never reports listener_ready; its
            // completion predicate is every owned Generation terminal.
            Ok(report) if report.decommission_complete || report.listener_ready => {
                (FleetObservationPhase::Ready, None)
            }
            Ok(_) => (FleetObservationPhase::Reconciling, None),
            Err(error) => (FleetObservationPhase::Degraded, Some(error.code)),
        };
        let observation = FleetObservation {
            guard: guard.clone(),
            session_epoch: result
                .as_ref()
                .map_or(captured_epoch, |report| report.session_epoch),
            phase,
            reason,
        };
        if let Some(listener) = &self.listener {
            listener.publish_observation(observation, now).await?;
        } else {
            self.store
                .fleet_set_observed(&self.config.fleet_key, &observation, now)
                .await?;
        }
        Ok(())
    }
}
