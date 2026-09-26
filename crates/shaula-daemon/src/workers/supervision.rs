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
                if !session.cleanup_attempted.swap(true, Ordering::AcqRel) {
                    use sha2::{Digest, Sha256};
                    let input = self.journal.protected_input(&session.access).await.ok();
                    let result = reaper
                        .reap(shaula_core::worker::cleanup::WorkspaceCleanup {
                            receipt: receipt.clone(),
                            workspace: session.workspace.clone(),
                            artifact: session.artifact.clone(),
                            artifact_digest: session.digest.clone(),
                            retained_input_digest: input
                                .map(|b| format!("sha256:{}", hex::encode(Sha256::digest(b)))),
                        })
                        .await;
                    if matches!(
                        result,
                        Ok(shaula_core::worker::cleanup::CleanupOutcome::Reaped)
                    ) && self.journal.workspace_reaped(&receipt).await.is_ok()
                    {
                        if let Some(identity) = session.identity.lock().await.clone() {
                            let _ = self.executor.release_fenced(&identity).await;
                        }
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
}
