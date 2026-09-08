//! Bounded display evidence from the supervisor's actual authorization check.
use shaula_core::error::CoreResult;
use shaula_core::ports::{AccessFailure, RouteProof};
use shaula_core::registry::AuthRouteObservation;

use super::FleetSupervisor;

impl FleetSupervisor {
    pub(super) async fn record_route_health(
        &self,
        proof: &Result<RouteProof, AccessFailure>,
        fallback_now: i64,
    ) -> CoreResult<()> {
        let Some(context) = self
            .handoff
            .auth_execution_context_get(
                &self.config.fleet_key,
                &self.config.auth_profile_key,
                self.config.auth_revision,
            )
            .await?
        else {
            return Ok(());
        };
        let (checked_at_ms, valid_until_ms, healthy, reason) = match proof {
            Ok(proof) => (
                proof.checked_at_unix_ms,
                proof.valid_until_unix_ms,
                true,
                None,
            ),
            Err(failure) => {
                let now = self
                    .clock
                    .as_ref()
                    .map_or(fallback_now, |clock| clock.now_unix_ms());
                let reason = match failure {
                    AccessFailure::Unauthenticated => "Unauthenticated",
                    AccessFailure::SessionExpired => "SessionExpired",
                    AccessFailure::PermissionDenied => "PermissionDenied",
                    AccessFailure::TargetHiddenOrNotFound => "TargetHiddenOrNotFound",
                    AccessFailure::RateLimited { .. } => "RateLimited",
                    AccessFailure::Unavailable { .. } => "AccessVerificationFailed",
                    AccessFailure::RequestUncertain { .. } => "RequestUncertain",
                };
                let (checked, until) = self
                    .github
                    .route_proof_failure_window(failure)
                    .await
                    .unwrap_or((now, now.saturating_add(15_000)));
                (checked, until, false, Some(reason.to_owned()))
            }
        };
        self.handoff
            .auth_route_observation_report(AuthRouteObservation {
                fleet_key: self.config.fleet_key.clone(),
                context,
                checked_at_ms,
                valid_until_ms,
                healthy,
                reason,
            })
            .await
    }
}
