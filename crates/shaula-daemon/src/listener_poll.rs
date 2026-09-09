use super::*;
use shaula_core::ports::{EffectOutcome, PollOutcome};
use shaula_core::registry::IngestedMessage;

impl FleetListener {
    /// One independently scheduled long poll. Polling holds no effect permit;
    /// only current-epoch ingestion, ACK and acquisition may take that permit.
    pub async fn poll_once(&self) -> CoreResult<()> {
        let epoch = self.state.lock().await.session.as_ref().map(|s| s.epoch);
        let result = self.poll_inner().await;
        if let (Err(error), Some(epoch)) = (&result, epoch) {
            if self
                .failed_session(
                    epoch,
                    &AccessFailure::Unavailable {
                        summary: error.code.as_str().into(),
                    },
                )
                .await
            {
                self.observe_failure(epoch, error.code).await?;
            }
        }
        result
    }

    async fn poll_inner(&self) -> CoreResult<()> {
        let remembered = {
            let state = self.state.lock().await;
            if !state.enabled || state.reconnect || self.deps.clock.now_unix_ms() < state.retry_at {
                return Ok(());
            }
            state.session.clone()
        };
        let Some(remembered) = remembered else {
            return Ok(());
        };
        if !self.current(remembered.epoch).await? {
            return Ok(());
        }
        let context = self.context(remembered.epoch);
        for message in self
            .deps
            .store
            .listener_pending(&self.config.fleet_key, &context)
            .await?
        {
            if !self.finish_message(&remembered, message).await? {
                return Ok(());
            }
        }
        let Some(session) = self.deps.store.session_get(&self.config.fleet_key).await? else {
            return Ok(());
        };
        if session.epoch != remembered.epoch {
            return Ok(());
        }
        let result = self
            .deps
            .github
            .poll_messages(
                &session.handle,
                session.last_message_id,
                self.config.max_capacity,
            )
            .await;
        match result {
            Ok(PollOutcome::NoMessage) => self.recovered(session.epoch).await?,
            Ok(PollOutcome::Message(message)) => {
                let committed = {
                    let _gate = self.deps.gates.acquire_claim(&self.config.fleet_key).await;
                    self.deps
                        .store
                        .listener_ingest(
                            &self.config.fleet_key,
                            &context,
                            &message,
                            self.deps.clock.now_unix_ms(),
                        )
                        .await?
                };
                if let Some(message) = committed {
                    if self.finish_message(&session, message).await? {
                        self.recovered(session.epoch).await?;
                    }
                }
            }
            Ok(PollOutcome::SessionExpired) | Err(AccessFailure::SessionExpired) => {
                self.poll_failed(session.epoch, AccessFailure::SessionExpired)
                    .await?;
            }
            Err(failure) => self.poll_failed(session.epoch, failure).await?,
        }
        Ok(())
    }

    async fn finish_message(
        &self,
        session: &PersistedSession,
        message: IngestedMessage,
    ) -> CoreResult<bool> {
        let _gate = self.deps.gates.acquire_claim(&self.config.fleet_key).await;
        let context = self.context(session.epoch);
        if !self.current(session.epoch).await? {
            return Ok(false);
        }
        if !message.acked {
            if let Err(failure) = self
                .deps
                .github
                .ack_message(&session.handle, message.message_id)
                .await
            {
                // A lost ACK reply can leave a remotely absent message. Renew
                // the session instead of treating a general 404 as success.
                if matches!(
                    failure,
                    AccessFailure::RequestUncertain { .. } | AccessFailure::TargetHiddenOrNotFound
                ) {
                    self.state.lock().await.reconnect = true;
                }
                self.poll_failed(session.epoch, failure).await?;
                return Ok(false);
            }
            if !self
                .deps
                .store
                .listener_acknowledge(
                    &self.config.fleet_key,
                    &context,
                    message.message_id,
                    self.deps.clock.now_unix_ms(),
                )
                .await?
            {
                return Ok(false);
            }
        }
        let Some(requests) = self
            .deps
            .store
            .listener_acquire_start(
                &self.config.fleet_key,
                &context,
                message.message_id,
                self.deps.clock.now_unix_ms(),
            )
            .await?
        else {
            return Ok(false);
        };
        if requests.is_empty() {
            return Ok(true);
        }
        let outcome = self
            .deps
            .github
            .acquire_jobs(session.scale_set_id, &session.handle, &requests)
            .await;
        let (accepted, failure) = match outcome {
            Ok(EffectOutcome::Definite(ids)) => (Some(ids), None),
            Ok(EffectOutcome::Uncertain { .. }) => (
                None,
                Some(AccessFailure::RequestUncertain {
                    summary: "acquisition outcome uncertain".into(),
                }),
            ),
            Err(failure) => {
                let accepted = if matches!(failure, AccessFailure::RequestUncertain { .. }) {
                    None
                } else {
                    Some(Vec::new())
                };
                (accepted, Some(failure))
            }
        };
        if accepted.is_none() {
            self.state.lock().await.reconnect = true;
        }
        let current = self
            .deps
            .store
            .listener_acquire_complete(
                &self.config.fleet_key,
                &context,
                message.message_id,
                accepted.as_deref(),
                self.deps.clock.now_unix_ms(),
            )
            .await?;
        if let Some(failure) = failure {
            self.poll_failed(session.epoch, failure).await?;
            return Ok(false);
        }
        Ok(current)
    }

    async fn poll_failed(&self, epoch: i64, failure: AccessFailure) -> CoreResult<()> {
        if self.current(epoch).await? && self.failed_session(epoch, &failure).await {
            self.observe_failure(epoch, access_reason(&failure)).await?;
        }
        Ok(())
    }

    async fn recovered(&self, epoch: i64) -> CoreResult<()> {
        if self.current(epoch).await? {
            let mut state = self.state.lock().await;
            if state.session.as_ref().map(|s| s.epoch) != Some(epoch) {
                return Ok(());
            }
            state.retry_at = 0;
            state.reason = None;
            state.failures = 0;
        }
        Ok(())
    }
}
