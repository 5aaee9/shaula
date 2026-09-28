//! Coordinates whole lifecycle workers. Existing supervisors retain CI safety
//! decisions; individual Terraform commands never cross this interface.
mod control;
mod launch;
mod logs;
mod session;
mod supervision;

use session::{Session, State};
use shaula_core::{
    ports::{
        DestroyClassification, TemplateCreateRequest, TemplateCreateResult, TemplateDestroyRequest,
        TemplateOutcomeError, TemplateRuntimePort,
    },
    state_backend::{StateError, StateResult},
    worker::{
        journal::WorkerJournal, wire::*, ControlAccess, Executor, FenceOutcome, MAX_CONTROL_BYTES,
    },
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

pub struct WorkerConfig {
    pub max_workers: usize,
    pub address: SocketAddr,
    pub executable_digest: String,
    pub engine: PathBuf,
    pub work_root: PathBuf,
    pub artifact_root: PathBuf,
}

pub struct Workers {
    config: WorkerConfig,
    journal: Arc<dyn WorkerJournal>,
    executor: Arc<dyn Executor>,
    sessions: Mutex<HashMap<Uuid, Arc<Session>>>,
    stopping: AtomicBool,
    launch_gate: Mutex<()>,
    capacity: Arc<tokio::sync::Semaphore>,
    logs: Option<Arc<dyn shaula_core::operation_log::OperationLogSink>>,
    log_reader: Option<Arc<dyn shaula_core::operation_log::OperationLogReadPort>>,
    reaper: Option<Arc<dyn shaula_core::worker::cleanup::WorkspaceReaper>>,
}

impl Workers {
    pub fn new(
        config: WorkerConfig,
        journal: Arc<dyn WorkerJournal>,
        executor: Arc<dyn Executor>,
    ) -> Self {
        Self {
            capacity: Arc::new(tokio::sync::Semaphore::new(config.max_workers)),
            config,
            journal,
            executor,
            sessions: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
            launch_gate: Mutex::new(()),
            logs: None,
            log_reader: None,
            reaper: None,
        }
    }

    pub fn with_logs(
        mut self,
        logs: Arc<dyn shaula_core::operation_log::OperationLogSink>,
        reader: Arc<dyn shaula_core::operation_log::OperationLogReadPort>,
    ) -> Self {
        self.logs = Some(logs);
        self.log_reader = Some(reader);
        self
    }

    pub fn with_reaper(
        mut self,
        reaper: Arc<dyn shaula_core::worker::cleanup::WorkspaceReaper>,
    ) -> Self {
        self.reaper = Some(reaper);
        self
    }

    async fn session(&self, id: Uuid) -> StateResult<Arc<Session>> {
        self.sessions
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or(StateError::Unauthorized)
    }

    async fn fence(&self, session: &Session) -> StateResult<()> {
        if session.fenced.load(Ordering::Acquire) {
            return Ok(());
        }
        let identity = session
            .identity
            .lock()
            .await
            .clone()
            .ok_or(StateError::Conflict)?;
        if self.executor.stop_and_fence(&identity).await != FenceOutcome::Fenced {
            return Err(StateError::Unavailable);
        }
        let mut state = session.state.lock().await;
        state.guard = None;
        state.bootstrap_guard = None;
        *session.permit.lock().await = None;
        drop(state);
        self.journal.fenced(&session.access, &identity).await?;
        session.fenced.store(true, Ordering::Release);
        let _ = self.executor.release_fenced(&identity).await;
        Ok(())
    }
}

fn failed() -> TemplateOutcomeError {
    TemplateOutcomeError::StateUnavailable {
        phase: "worker.unavailable".into(),
    }
}

#[async_trait::async_trait]
impl TemplateRuntimePort for Workers {
    async fn prepare_create(
        &self,
        workspace: &Path,
        artifact: &Path,
        digest: &str,
        timeout: Duration,
    ) -> Result<(), TemplateOutcomeError> {
        self.prepare(workspace, artifact, digest, timeout)
            .await
            .map_err(|_| failed())?
    }

    async fn create(
        &self,
        request: TemplateCreateRequest,
    ) -> Result<TemplateCreateResult, TemplateOutcomeError> {
        let id = Uuid::parse_str(&request.input.generation.id).map_err(|_| failed())?;
        let session = self.session(id).await.map_err(|_| failed())?;
        session
            .check_material(
                &request.workspace_path,
                &request.artifact_dir,
                &request.pinned_artifact_digest,
            )
            .map_err(|_| failed())?;
        let (result, receiver) = oneshot::channel();
        self.journal
            .retain_input(
                &session.access,
                request
                    .input
                    .to_tfvars()
                    .map_err(|_| failed())?
                    .into_bytes(),
            )
            .await
            .map_err(|_| failed())?;
        {
            let mut state = session.state.lock().await;
            if state.created || state.desired.is_some() || !state.prepared_ok {
                return Err(failed());
            }
            let reference = state
                .material(&CreateMaterial::from_request(&request))
                .map_err(|_| failed())?;
            state.created = true;
            state.sink = request.apply_intent_sink;
            state.created_result = Some(result);
            state.desired = Some(Directive::Create(reference));
        }
        match self.receive(&session, receiver, request.timeout).await {
            Ok(result) => result,
            _ => {
                let _ = self.fence(&session).await;
                Err(failed())
            }
        }
    }

    async fn destroy(
        &self,
        request: TemplateDestroyRequest,
    ) -> Result<DestroyClassification, TemplateOutcomeError> {
        let id = Uuid::parse_str(&request.generation_id).map_err(|_| failed())?;
        self.ensure_replacement(id).await.map_err(|_| failed())?;
        if self.session(id).await.is_err() {
            self.prepare(
                &request.workspace_path,
                &request.artifact_dir,
                &request.pinned_artifact_digest,
                request.timeout,
            )
            .await
            .map_err(|_| failed())??;
        }
        let session = self.session(id).await.map_err(|_| failed())?;
        session
            .check_material(
                &request.workspace_path,
                &request.artifact_dir,
                &request.pinned_artifact_digest,
            )
            .map_err(|_| failed())?;
        let (result, receiver) = oneshot::channel();
        {
            let mut state = session.state.lock().await;
            if state.desired.is_some() {
                return Err(failed());
            }
            let mut material = DestroyMaterial::from_request(&request);
            material.protected_input = Some(
                String::from_utf8(
                    self.journal
                        .protected_input(&session.access)
                        .await
                        .map_err(|_| failed())?
                        .ok_or_else(failed)?,
                )
                .map_err(|_| failed())?,
            );
            let reference = state.material(&material).map_err(|_| failed())?;
            state.sink = request.apply_intent_sink;
            state.destroyed_result = Some(result);
            state.desired = Some(Directive::Destroy(reference));
        }
        match self.receive(&session, receiver, request.timeout).await {
            Ok(result) => result,
            _ => {
                let _ = self.fence(&session).await;
                Err(failed())
            }
        }
    }
}
