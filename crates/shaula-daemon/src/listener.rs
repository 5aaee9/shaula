//! Per-Fleet session ownership and message processing, separate from capacity work.

use std::collections::HashMap;
use std::sync::Arc;

use shaula_core::auth_context::ResolvedAuthContext;
use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::ports::{AccessFailure, Clock, GitHubAccessPort};
use shaula_core::registry::{
    FleetRuntimeGuard, LifecycleStore, PersistedSession, SessionEffectContext,
};
use tokio::sync::Mutex;

use crate::effect_gate::FleetEffectGates;

pub struct ListenerConfig {
    pub fleet_key: String,
    pub guard: FleetRuntimeGuard,
    pub auth_context: ResolvedAuthContext,
    pub max_capacity: i64,
}

pub struct ListenerDeps {
    pub store: Arc<dyn LifecycleStore>,
    pub github: Arc<dyn GitHubAccessPort>,
    pub retained_clients: HashMap<(String, i64), Arc<dyn GitHubAccessPort>>,
    pub gates: Arc<FleetEffectGates>,
    pub clock: Arc<dyn Clock>,
}

#[derive(Default)]
struct ListenerState {
    session: Option<PersistedSession>,
    uncommitted_session: Option<(i64, shaula_core::ports::SessionHandle)>,
    retry_at: i64,
    failures: u32,
    reason: Option<ReasonCode>,
    enabled: bool,
    reconnect: bool,
    stopped: bool,
}

pub struct FleetListener {
    config: ListenerConfig,
    deps: ListenerDeps,
    state: Mutex<ListenerState>,
}

impl FleetListener {
    pub fn new(config: ListenerConfig, deps: ListenerDeps) -> Self {
        Self {
            config,
            deps,
            state: Mutex::default(),
        }
    }

    pub fn matches(&self, guard: &FleetRuntimeGuard, context: &ResolvedAuthContext) -> bool {
        self.config.guard == *guard && self.config.auth_context == *context
    }

    fn context(&self, epoch: i64) -> SessionEffectContext {
        SessionEffectContext {
            guard: self.config.guard.clone(),
            auth_context: self.config.auth_context.clone(),
            epoch,
        }
    }

    pub async fn reason(&self) -> Option<ReasonCode> {
        self.state.lock().await.reason
    }

    pub(crate) async fn handoff_gate(&self) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.deps
            .gates
            .acquire_exclusive(&self.config.fleet_key)
            .await
    }

    /// Recheck the complete Fleet/Auth authority while holding the effect gate.
    pub(crate) async fn authorize_runtime(&self) -> CoreResult<bool> {
        self.deps
            .store
            .session_authorize(
                &self.config.fleet_key,
                &self.config.guard,
                &self.config.auth_context,
            )
            .await
    }

    async fn current(&self, epoch: i64) -> CoreResult<bool> {
        self.deps
            .store
            .listener_current(&self.config.fleet_key, &self.context(epoch))
            .await
    }

    async fn failed(&self, failure: &AccessFailure) {
        self.set_failure(failure, None).await;
    }

    async fn failed_session(&self, epoch: i64, failure: &AccessFailure) -> bool {
        self.set_failure(failure, Some(epoch)).await
    }

    async fn set_failure(&self, failure: &AccessFailure, epoch: Option<i64>) -> bool {
        let mut state = self.state.lock().await;
        if epoch.is_some() && state.session.as_ref().map(|s| s.epoch) != epoch {
            return false;
        }
        if matches!(failure, AccessFailure::SessionExpired) {
            state.reconnect = true;
            state.enabled = false;
        }
        state.failures = state.failures.saturating_add(1);
        let seconds = 5_i64.saturating_mul(1_i64 << state.failures.min(5));
        let retry_ms = match failure {
            AccessFailure::RateLimited {
                retry_after: Some(delay),
            } => i64::try_from(delay.as_millis())
                .unwrap_or(i64::MAX)
                .max(seconds * 1000),
            _ => seconds * 1000,
        };
        // Fleet-specific jitter keeps independent reconnects from synchronizing.
        let jitter = self
            .config
            .fleet_key
            .bytes()
            .fold(0_i64, |n, b| (n + i64::from(b)) % 1000);
        state.retry_at = self
            .deps
            .clock
            .now_unix_ms()
            .saturating_add(retry_ms)
            .saturating_add(jitter);
        state.reason = Some(access_reason(failure));
        true
    }
}

pub(crate) fn access_reason(failure: &AccessFailure) -> ReasonCode {
    match failure {
        AccessFailure::Unauthenticated | AccessFailure::SessionExpired => {
            ReasonCode::Unauthenticated
        }
        AccessFailure::PermissionDenied => ReasonCode::PermissionDenied,
        AccessFailure::TargetHiddenOrNotFound => ReasonCode::TargetHiddenOrNotFound,
        AccessFailure::RateLimited { .. } => ReasonCode::RateLimited,
        AccessFailure::Unavailable { .. } | AccessFailure::RequestUncertain { .. } => {
            ReasonCode::AccessVerificationFailed
        }
    }
}

#[path = "listener_poll.rs"]
mod poll;
#[path = "listener_session.rs"]
mod session;
