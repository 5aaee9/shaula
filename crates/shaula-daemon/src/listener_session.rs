use super::*;
use shaula_core::ports::EffectOutcome;
use shaula_core::registry::{FleetObservation, FleetObservationPhase, SessionInstall};

impl FleetListener {
    /// Session replacement drains ACK/acquire effects, then atomically installs
    /// the full handle, its initial statistics and its exact authority.
    pub async fn ensure_session(&self, scale_set_id: i64) -> CoreResult<Option<i64>> {
        let _gate = self
            .deps
            .gates
            .acquire_exclusive(&self.config.fleet_key)
            .await;
        let now = self.deps.clock.now_unix_ms();
        if now < self.state.lock().await.retry_at {
            return Ok(None);
        }
        if !self.clear_uncommitted_locked().await {
            return Ok(None);
        }
        if !self
            .deps
            .store
            .session_authorize(
                &self.config.fleet_key,
                &self.config.guard,
                &self.config.auth_context,
            )
            .await?
        {
            return Ok(None);
        }
        {
            let state = self.state.lock().await;
            // A concurrent poll can fail while authorization is being read.
            // Recheck the deadline together with the session/reconnect state.
            if now < state.retry_at {
                return Ok(None);
            }
            if let Some(session) = &state.session {
                if !state.reconnect
                    && session.scale_set_id == scale_set_id
                    && self.current(session.epoch).await?
                {
                    let epoch = session.epoch;
                    drop(state);
                    self.state.lock().await.enabled = true;
                    return Ok(Some(epoch));
                }
            }
        }
        if !self.clear_session_locked(true).await? {
            return Ok(None);
        }
        if !self
            .deps
            .store
            .session_authorize(
                &self.config.fleet_key,
                &self.config.guard,
                &self.config.auth_context,
            )
            .await?
        {
            return Ok(None);
        }
        let expected_epoch = self
            .deps
            .store
            .session_epoch(&self.config.fleet_key)
            .await?;
        let owner = format!(
            "shaula-{}-{}",
            self.config.guard.incarnation,
            expected_epoch.unwrap_or(0)
        );
        let result = self
            .deps
            .github
            .establish_session(scale_set_id, &owner)
            .await;
        let handle = match result {
            Ok(EffectOutcome::Definite(handle)) => handle,
            Ok(EffectOutcome::Uncertain { .. }) => {
                self.failed(&AccessFailure::RequestUncertain {
                    summary: "session establishment uncertain".into(),
                })
                .await;
                return Ok(None);
            }
            Err(failure) => {
                self.failed(&failure).await;
                return Ok(None);
            }
        };
        let install = SessionInstall {
            guard: self.config.guard.clone(),
            expected_epoch,
            auth_context: self.config.auth_context.clone(),
            scale_set_id,
            handle,
        };
        self.state.lock().await.uncommitted_session = Some((scale_set_id, install.handle.clone()));
        let epoch = self
            .deps
            .store
            .session_install(&self.config.fleet_key, &install, now)
            .await;
        let epoch = match epoch {
            Ok(Some(epoch)) => epoch,
            result => {
                // Both a failed transaction and a lost CAS leave a remote
                // session to close. Retain its exact handle if cleanup fails;
                // the next reconcile must clean it before another POST.
                self.clear_uncommitted_locked().await;
                return result;
            }
        };
        let mut state = self.state.lock().await;
        state.uncommitted_session = None;
        state.session = Some(PersistedSession {
            epoch,
            scale_set_id,
            last_message_id: 0,
            auth_context: self.config.auth_context.clone(),
            handle: install.handle,
        });
        state.enabled = true;
        state.stopped = false;
        state.reconnect = false;
        state.reason = None;
        state.failures = 0;
        state.retry_at = 0;
        Ok(Some(epoch))
    }

    async fn clear_uncommitted_locked(&self) -> bool {
        let candidate = self.state.lock().await.uncommitted_session.clone();
        let Some((scale_set_id, handle)) = candidate else {
            return true;
        };
        if let Err(failure) = self
            .deps
            .github
            .delete_session(scale_set_id, &handle.session_id)
            .await
        {
            if !matches!(failure, AccessFailure::SessionExpired) {
                self.failed(&failure).await;
                return false;
            }
        }
        self.state.lock().await.uncommitted_session = None;
        true
    }

    async fn clear_session_locked(&self, replace: bool) -> CoreResult<bool> {
        self.state.lock().await.enabled = false;
        let stored = self.deps.store.session_get(&self.config.fleet_key).await?;
        let remembered = self.state.lock().await.session.clone();
        // A retired listener may only close its own remembered handle. A
        // current reconciler may replace a persisted predecessor on restart.
        let session = if replace {
            stored.or(remembered)
        } else if remembered.is_none()
            && self
                .deps
                .store
                .session_close_authorize(&self.config.fleet_key, &self.config.guard)
                .await?
        {
            stored
        } else {
            remembered
        };
        let Some(session) = session else {
            return Ok(true);
        };
        let reference = (
            session.auth_context.profile_key.clone(),
            session.auth_context.revision,
        );
        let client = if session.auth_context == self.config.auth_context {
            Some(&self.deps.github)
        } else {
            self.deps.retained_clients.get(&reference)
        };
        let Some(client) = client else {
            self.state.lock().await.reason = Some(ReasonCode::OwnershipProofFailed);
            return Ok(false);
        };
        if let Err(failure) = client
            .delete_session(session.scale_set_id, &session.handle.session_id)
            .await
        {
            // Authenticated absent session is already stopped; permissions and
            // transport failures retain the session and its recovery evidence.
            if !matches!(failure, AccessFailure::SessionExpired) {
                self.failed(&failure).await;
                return Ok(false);
            }
        }
        self.deps
            .store
            .session_clear(
                &self.config.fleet_key,
                session.epoch,
                self.deps.clock.now_unix_ms(),
            )
            .await?;
        self.state.lock().await.session = None;
        Ok(true)
    }

    pub async fn stop(&self) -> CoreResult<bool> {
        let _gate = self
            .deps
            .gates
            .acquire_exclusive(&self.config.fleet_key)
            .await;
        if !self.clear_uncommitted_locked().await {
            return Ok(false);
        }
        if self.state.lock().await.stopped {
            return Ok(true);
        }
        let stopped = self.clear_session_locked(false).await?;
        self.state.lock().await.stopped = stopped;
        Ok(stopped)
    }

    pub async fn observe_failure(&self, epoch: i64, reason: ReasonCode) -> CoreResult<()> {
        let state = self.state.lock().await;
        if state.reason.is_none() {
            return Ok(());
        }
        self.deps
            .store
            .fleet_set_observed(
                &self.config.fleet_key,
                &FleetObservation {
                    guard: self.config.guard.clone(),
                    session_epoch: Some(epoch),
                    phase: FleetObservationPhase::Degraded,
                    reason: Some(reason),
                },
                self.deps.clock.now_unix_ms(),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn publish_observation(
        &self,
        mut observation: FleetObservation,
        now: i64,
    ) -> CoreResult<()> {
        // Serialize Ready with failure publication. A slow capacity reconcile
        // must not erase a newer listener failure on the same session epoch.
        let state = self.state.lock().await;
        if observation.phase == FleetObservationPhase::Ready {
            if let Some(reason) = state.reason {
                observation.phase = FleetObservationPhase::Degraded;
                observation.reason = Some(reason);
            }
        }
        self.deps
            .store
            .fleet_set_observed(&self.config.fleet_key, &observation, now)
            .await?;
        Ok(())
    }
}
