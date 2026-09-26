use super::*;
use shaula_core::worker::ProcessObservation;

impl Workers {
    pub(super) async fn receive<T>(
        &self,
        session: &Session,
        mut receiver: oneshot::Receiver<T>,
        timeout: Duration,
    ) -> StateResult<T> {
        let deadline = tokio::time::sleep(timeout + Duration::from_secs(80));
        tokio::pin!(deadline);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                result = &mut receiver => return result.map_err(|_| StateError::Unavailable),
                _ = &mut deadline => return Err(StateError::Unavailable),
                _ = tick.tick() => {
                    let identity = session.identity.lock().await.clone().ok_or(StateError::Conflict)?;
                    if self.executor.observe(&identity).await != ProcessObservation::Live { return Err(StateError::Unavailable); }
                }
            }
        }
    }
    pub(super) async fn ensure_replacement(&self, id: Uuid) -> StateResult<()> {
        let Ok(session) = self.session(id).await else {
            return Ok(());
        };
        let identity = session
            .identity
            .lock()
            .await
            .clone()
            .ok_or(StateError::Conflict)?;
        if self.executor.observe(&identity).await == ProcessObservation::Live {
            return Ok(());
        }
        self.fence(&session).await?;
        self.journal.replace_fenced(&session.access).await?;
        self.sessions.lock().await.remove(&id);
        Ok(())
    }

    /// Polls are bounded by max_workers plus a 60-second receipt replay window.
    /// Waiting workers hold no Terraform mutation permit. State remains online
    /// until this supervisor and every executor containment have stopped.
    pub async fn supervise(
        &self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> StateResult<()> {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = shutdown.changed() => return Ok(()),
                _ = tick.tick() => self.reap_completed().await?,
            }
        }
    }

    async fn reap_completed(&self) -> StateResult<()> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|(id, session)| (*id, session.clone()))
            .collect();
        for (id, session) in sessions {
            let Some(receipt) = self.journal.receipt(&session.access).await? else {
                continue;
            };
            let mut completed = session.completed.lock().await;
            let since = *completed.get_or_insert_with(tokio::time::Instant::now);
            if since.elapsed() < Duration::from_secs(1) {
                continue;
            }
            if let Some(identity) = session.identity.lock().await.clone() {
                if !session.fenced.load(Ordering::Acquire)
                    && self.executor.stop_and_fence(&identity).await != FenceOutcome::Fenced
                {
                    return Err(StateError::Unavailable);
                }
                session.fenced.store(true, Ordering::Release);
            }
            *session.permit.lock().await = None;
            if let Some(reaper) = &self.reaper {
                if session.cleanup_attempts.load(Ordering::Acquire) < 3 {
                    let attempt = session.cleanup_attempts.fetch_add(1, Ordering::AcqRel) + 1;
                    match self
                        .reap_workspace(&session, &receipt, reaper.as_ref())
                        .await
                    {
                        Ok(()) => session.cleanup_attempts.store(3, Ordering::Release),
                        Err(_) => tracing::warn!(generation_id = %id, attempt,
                            "receipt-authorized workspace cleanup deferred; evidence retained"),
                    }
                }
            }
            // Keep receipt authority briefly; no state writes can reopen the
            // sealed backend. Unknown/emergency workspace files are retained.
            if since.elapsed() >= Duration::from_secs(60) {
                self.sessions.lock().await.remove(&id);
            }
        }
        Ok(())
    }

    async fn reap_workspace(
        &self,
        session: &Session,
        receipt: &shaula_core::worker::CompletionReceipt,
        reaper: &dyn shaula_core::worker::cleanup::WorkspaceReaper,
    ) -> StateResult<()> {
        use sha2::{Digest, Sha256};
        use shaula_core::worker::{cleanup::*, CompletionKind};
        let input = self.journal.protected_input(&session.access).await?;
        if receipt.kind == CompletionKind::ProviderCleanup && input.is_none() {
            return Err(StateError::Unavailable);
        }
        let result = reaper
            .reap(WorkspaceCleanup {
                receipt: receipt.clone(),
                workspace: session.workspace.clone(),
                artifact: session.artifact.clone(),
                artifact_digest: session.digest.clone(),
                retained_input_digest: input
                    .map(|b| format!("sha256:{}", hex::encode(Sha256::digest(b)))),
            })
            .await?;
        if result == CleanupOutcome::Reaped {
            self.journal.workspace_reaped(receipt).await?;
            if let Some(identity) = session.identity.lock().await.clone() {
                self.executor.release_fenced(&identity).await?;
            }
        }
        Ok(())
    }
}
