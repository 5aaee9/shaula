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
        let Some(fleet) = self.store.fleet_get(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        if fleet.tombstone {
            return Ok(Err(MutationError::Gone {
                tombstone: key.to_string(),
            }));
        }
        let Some(revision) = self.store.fleet_revision_latest(key).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        let spec: FleetSpec = serde_json::from_str(&revision.spec_json)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        Ok(Ok(FleetResource {
            key: key.to_string(),
            spec,
            incarnation: fleet.incarnation,
            revision: fleet.desired_revision,
            resolved_template: revision.template_profile_key.clone().map(|k| {
                (
                    k,
                    revision.template_revision.unwrap_or_default(),
                    revision
                        .template_artifact_digest
                        .clone()
                        .unwrap_or_default(),
                    revision.template_attestation_id.clone().unwrap_or_default(),
                )
            }),
            resolved_template_pool: revision.template_pool.clone(),
            resolved_template_pool_ref: revision.template_pool_ref.clone(),
            resolved_auth: revision.auth_desired,
            created_at: revision.created_at,
            updated_at: revision.created_at,
        }))
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
