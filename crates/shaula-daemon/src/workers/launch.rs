use super::*;

impl Workers {
    /// Stop admission first. Backend/control stay available while in-flight
    /// operations finish. Deadline expiry requires actual containment fencing.
    pub async fn shutdown(&self, deadline: Duration) -> StateResult<()> {
        self.stopping.store(true, Ordering::Release);
        self.capacity.close();
        let _launch = self.launch_gate.lock().await;
        let sessions: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        let until = tokio::time::Instant::now() + deadline;
        let outcomes = futures::future::join_all(sessions.into_iter().map(|session| async move {
            while tokio::time::Instant::now() < until {
                let Some(identity) = session.identity.lock().await.clone() else {
                    break;
                };
                if self.executor.observe(&identity).await
                    == shaula_core::worker::ProcessObservation::Exited
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if session.identity.lock().await.is_some() {
                if self.journal.receipt(&session.access).await?.is_none() {
                    self.fence(&session).await?;
                } else if let Some(identity) = session.identity.lock().await.clone() {
                    if !session.fenced.load(Ordering::Acquire)
                        && self.executor.stop_and_fence(&identity).await != FenceOutcome::Fenced
                    {
                        return Err(StateError::Unavailable);
                    }
                }
            }
            Ok(())
        }))
        .await;
        outcomes
            .into_iter()
            .collect::<StateResult<Vec<()>>>()
            .map(|_| ())
    }

    pub(super) async fn prepare(
        &self,
        workspace: &Path,
        artifact: &Path,
        digest: &str,
        timeout: Duration,
    ) -> StateResult<Result<(), TemplateOutcomeError>> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(StateError::Unavailable);
        }
        let id = workspace
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or(StateError::Invalid)?;
        let permit = self
            .capacity
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StateError::Unavailable)?;
        let launch = self.launch_gate.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err(StateError::Unavailable);
        }
        {
            let mut sessions = self.sessions.lock().await;
            if sessions.len() >= self.config.max_workers * 2 {
                sessions.retain(|_, session| {
                    !session.fenced.load(Ordering::Acquire)
                        || session
                            .completed
                            .try_lock()
                            .map_or(true, |complete| complete.is_none())
                });
            }
            if sessions.len() >= self.config.max_workers * 2 {
                return Err(StateError::Unavailable);
            }
        }
        let admission = self.journal.admission(id).await?;
        let access = ControlAccess {
            claim: admission.claim.clone(),
            capability: admission.control.clone(),
        };
        let (prepared, receiver) = oneshot::channel();
        let session = Arc::new(Session {
            access,
            workspace: workspace.to_owned(),
            artifact: artifact.to_owned(),
            digest: digest.to_owned(),
            identity: Mutex::new(None),
            state: Mutex::new(State::new(prepared)),
            logs: Mutex::new(Default::default()),
            permit: Mutex::new(Some(permit)),
            completed: Mutex::new(None),
            cleanup_attempted: AtomicBool::new(false),
            fenced: AtomicBool::new(false),
        });
        {
            let mut sessions = self.sessions.lock().await;
            if self.stopping.load(Ordering::Acquire) {
                return Err(StateError::Unavailable);
            }
            if sessions.contains_key(&id) {
                return Err(StateError::Conflict);
            }
            sessions.insert(id, session.clone());
        }
        self.journal.launch_pending(&session.access).await?;
        let identity = self.executor.launch(&admission.claim).await?;
        *session.identity.lock().await = Some(identity.clone());
        if let Err(error) = self.journal.register(&session.access, &identity).await {
            // No envelope was released. Still fence the exact tree before
            // returning; a failed/uncertain journal response is not a fence.
            let _ = self.executor.stop_and_fence(&identity).await;
            return Err(error);
        }
        let handoff = self
            .executor
            .handoff(LaunchEnvelope {
                protocol: shaula_core::worker::PROTOCOL_VERSION,
                claim: admission.claim,
                address: self.config.address,
                control_capability: admission.control.expose().to_owned(),
                state_capability: admission.state.expose().to_owned(),
                executable_digest: self.config.executable_digest.clone(),
                engine: self.config.engine.clone(),
                work_root: self.config.work_root.clone(),
                artifact_root: self.config.artifact_root.clone(),
                workspace: workspace.to_owned(),
                artifact: artifact.to_owned(),
                artifact_digest: digest.to_owned(),
                operation_timeout: timeout,
                cleanup_only: admission.cleanup_only,
            })
            .await;
        if let Err(error) = handoff {
            self.fence(&session).await?;
            return Err(error);
        }
        drop(launch);
        match self.receive(&session, receiver, timeout).await {
            Ok(result) => {
                if result.is_err() {
                    self.fence(&session).await?;
                }
                Ok(result)
            }
            _ => {
                self.fence(&session).await?;
                Err(StateError::Unavailable)
            }
        }
    }
}
