//! Exact execution authority reads.
use super::{core_err, SqliteControlPlane};
use shaula_core::error::CoreResult;

#[async_trait::async_trait]
impl shaula_core::registry::AuthExecutionStore for SqliteControlPlane {
    async fn auth_route_observation_report(
        &self,
        observation: shaula_core::registry::AuthRouteObservation,
    ) -> CoreResult<()> {
        self.auth_observations.report(observation);
        Ok(())
    }
    async fn auth_route_observations(
        &self,
        key: &str,
        revision: i64,
        now_ms: i64,
    ) -> CoreResult<Vec<shaula_core::registry::AuthRouteObservation>> {
        Ok(self.auth_observations.read(key, revision, now_ms))
    }

    async fn auth_execution_refs(&self, fleet_key: &str) -> CoreResult<Vec<(String, i64)>> {
        self.store
            .auth_execution_refs(fleet_key)
            .await
            .map_err(core_err)
    }
    async fn auth_execution_context_get(
        &self,
        fleet_key: &str,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<shaula_core::auth_context::ResolvedAuthContext>> {
        self.store
            .auth_execution_context_get(fleet_key, key, revision)
            .await
            .map_err(core_err)
    }
    async fn auth_generation_ref(&self, generation_id: &str) -> CoreResult<Option<(String, i64)>> {
        self.store
            .auth_generation_ref(generation_id)
            .await
            .map_err(core_err)
    }
}
