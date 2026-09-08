//! Retained execution authority and transient display observations.
use crate::error::CoreResult;

/// The last actual route access check for an exact Fleet revision context.
/// This is UI evidence only; expiration or restart always means Unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRouteObservation {
    pub fleet_key: String,
    pub context: crate::auth_context::ResolvedAuthContext,
    pub checked_at_ms: i64,
    pub valid_until_ms: i64,
    pub healthy: bool,
    /// Stable reason code, never a remote error body or credential.
    pub reason: Option<String>,
}

/// Read seam for exact execution authority retained for cleanup and recovery.
#[async_trait::async_trait]
pub trait AuthExecutionStore: Send + Sync {
    /// Display-only bounded observations, never an authorization authority.
    async fn auth_route_observation_report(
        &self,
        _observation: AuthRouteObservation,
    ) -> CoreResult<()> {
        Ok(())
    }
    async fn auth_route_observations(
        &self,
        _key: &str,
        _revision: i64,
        _now_ms: i64,
    ) -> CoreResult<Vec<AuthRouteObservation>> {
        Ok(Vec::new())
    }

    /// Actual observed, generation and session references, excluding unused history.
    async fn auth_execution_refs(&self, fleet_key: &str) -> CoreResult<Vec<(String, i64)>>;
    /// Immutable observed authority retained for an exact execution revision.
    async fn auth_execution_context_get(
        &self,
        fleet_key: &str,
        key: &str,
        revision: i64,
    ) -> CoreResult<Option<crate::auth_context::ResolvedAuthContext>>;
    /// The authority frozen at JIT admission; legacy rows use their original Fleet revision.
    async fn auth_generation_ref(&self, generation_id: &str) -> CoreResult<Option<(String, i64)>>;
}
