//! Fleet Registry port implementation on the control-plane service.

use async_trait::async_trait;

use super::conditional_put::{fleet::Fleet, Request};
use super::ControlPlane;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::fleet::FleetSpec;
use shaula_core::registry::{
    Actor, ChangeView, FleetRegistryPort, FleetResource, FleetStatus, HealthPort, MutationAccepted,
    MutationError,
};

#[async_trait]
impl HealthPort for ControlPlane {
    async fn live(&self) -> bool {
        true
    }
    async fn ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl FleetRegistryPort for ControlPlane {
    async fn diagnostics(
        &self,
        actor: &Actor,
        kind: shaula_core::diagnostics::SubjectKind,
        key: &str,
    ) -> shaula_core::diagnostics::DiagnosticsResult {
        self.store
            .diagnostic_read(kind, key, actor, self.now_ms())
            .await
    }

    #[tracing::instrument(name = "shaula.registry.fleet_put", skip_all, fields(key = %key))]
    async fn fleet_put(
        &self,
        actor: &Actor,
        key: &str,
        spec: FleetSpec,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.conditional_put(
            Request {
                actor,
                key,
                create: if_none_match,
                expected: if_match,
                idempotency_key,
            },
            || Fleet::prepare(spec),
        )
        .await
    }

    async fn fleet_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<FleetResource, MutationError>> {
        self.fleet_get_read(_actor, key).await
    }

    async fn fleet_status_get(
        &self,
        _actor: &Actor,
        key: &str,
    ) -> CoreResult<Result<FleetStatus, MutationError>> {
        self.fleet_status_impl(key).await
    }

    async fn fleet_list(&self, _actor: &Actor) -> CoreResult<Vec<(String, i64, String)>> {
        self.store.fleet_list(_actor).await
    }

    async fn fleet_delete(
        &self,
        actor: &Actor,
        key: &str,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.fleet_delete_impl(actor, key, if_match, idempotency_key)
            .await
    }

    async fn fleet_change_get(
        &self,
        _actor: &Actor,
        change_id: &str,
    ) -> CoreResult<Option<ChangeView>> {
        self.store.fleet_change_get(change_id).await
    }

    async fn generation_finalize(
        &self,
        actor: &Actor,
        generation_id: &str,
        reason: &str,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.generation_finalize_impl(actor, generation_id, reason, idempotency_key)
            .await
    }
}

#[path = "service_fleet_read.rs"]
mod read;
