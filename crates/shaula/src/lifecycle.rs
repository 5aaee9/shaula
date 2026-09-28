//! Production composition of the private listener, durable journal and exec.
use shaula_core::worker::{Executor, FenceOutcome};
use shaula_daemon::{
    bootstrap::ValidatedBootstrap,
    workers::{WorkerConfig, Workers},
};
use shaula_store::{
    http_state::{SqliteStateBackend, SqliteWorkerJournal, WorkerAdmissions},
    Store,
};
use std::{sync::Arc, time::Duration};

pub(crate) struct Lifecycle {
    pub workers: Arc<Workers>,
    pub admissions: Arc<WorkerAdmissions>,
    pub migration_required: bool,
    pub server: tokio::task::JoinHandle<std::io::Result<()>>,
    stop: tokio::sync::oneshot::Sender<()>,
}

impl Lifecycle {
    pub async fn start(
        store: Store,
        bootstrap: &ValidatedBootstrap,
        logs: Option<Arc<shaula_store::operation_logs::OperationLogArchive>>,
    ) -> Result<Self, String> {
        let config = bootstrap.lifecycle.as_ref().ok_or(
            "lifecycle configuration with explicit max_workers and recovery_reserve is required",
        )?;
        let executor = Arc::new(
            shaula_executor::ExecExecutor::new(
                std::env::current_exe().map_err(|_| "exact executable unavailable")?,
                match &config.cgroup_root {
                    Some(root) => root.clone(),
                    None => shaula_executor::ExecExecutor::delegated_root()
                        .map_err(|_| "delegated cgroup v2 required")?,
                },
            )
            .map_err(|_| {
                "exec requires Linux with a writable delegated cgroup v2 supporting cgroup.kill"
            })?,
        );
        let backend = Arc::new(SqliteStateBackend::new(store));
        let mut migration_required = false;
        if !backend
            .activated()
            .await
            .map_err(|_| "deployment format unavailable")?
        {
            match backend.activate().await {
                Ok(()) => {}
                Err(shaula_core::state_backend::StateError::Conflict) => migration_required = true,
                Err(_) => return Err("deployment activation unavailable".into()),
            }
        }
        let admissions = Arc::new(
            WorkerAdmissions::new(config.max_workers, config.recovery_reserve)
                .map_err(|_| "invalid worker limits")?,
        );
        let server =
            shaula_http::state_backend::StateServer::bind(config.internal_listen, backend.clone())
                .await
                .map_err(|_| "private listener bind failed")?;
        let address = server
            .local_addr()
            .map_err(|_| "private listener address unavailable")?;
        let journal = Arc::new(SqliteWorkerJournal::new(
            (*backend).clone(),
            admissions.clone(),
        ));
        let reaper = Arc::new(shaula_template::cleanup::ReceiptWorkspaceReaper::new(
            bootstrap.work_root.clone(),
            bootstrap.artifact_root.clone(),
        ));
        let mut workers = Workers::new(
            WorkerConfig {
                max_workers: config.max_workers,
                address,
                executable_digest: executor.executable_digest().to_owned(),
                engine: bootstrap.terraform_executable.clone(),
                work_root: std::fs::canonicalize(&bootstrap.work_root)
                    .map_err(|_| "work root unavailable")?,
                artifact_root: std::fs::canonicalize(&bootstrap.artifact_root)
                    .map_err(|_| "artifact root unavailable")?,
            },
            journal,
            executor.clone(),
        )
        .with_reaper(reaper.clone());
        if let Some(logs) = logs {
            workers = workers.with_logs(logs.clone(), logs);
        }
        let workers = Arc::new(workers);
        let server = server.with_worker_control(workers.clone());
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(server.serve(async {
            let _ = shutdown.await;
        }));
        // A bound socket precedes handoff. A request verifies the serving task,
        // including routing, before any Fleet acquisition is launched.
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| "private readiness client failed")?;
        client
            .get(format!("http://{address}/internal/ready"))
            .send()
            .await
            .map_err(|_| "private listener not serving")?;
        let records = if migration_required {
            Vec::new()
        } else {
            backend
                .recovery_records()
                .await
                .map_err(|_| "worker recovery scan failed")?
        };
        for record in records {
            let outcome = match record.identity() {
                Ok(_) if record.phase == "fenced" => FenceOutcome::Fenced,
                Ok(Some(identity)) => executor.stop_and_fence(&identity).await,
                Ok(None) if matches!(record.phase.as_str(), "admitted" | "fenced") => {
                    FenceOutcome::Fenced
                }
                // Pre-exec crash: classified by the exact containment, never
                // by an empty in-memory child list.
                Ok(None) if record.phase == "launch_pending" => match record.claim() {
                    Ok(claim) => executor.fence_unregistered(&claim).await,
                    Err(_) => FenceOutcome::Unknown,
                },
                _ => FenceOutcome::Unknown,
            };
            let workspace = std::path::Path::new(&record.workspace_path);
            let emergency = [
                "errored.tfstate",
                "terraform.tfstate",
                "terraform.tfstate.backup",
            ]
            .iter()
            .any(|name| workspace.join(name).exists());
            backend
                .recover_worker(&record, outcome, &admissions, emergency)
                .await
                .map_err(|_| "worker recovery classification failed")?;
        }
        if !migration_required {
            crate::lifecycle_cleanup::reap(
                &backend,
                executor.as_ref(),
                reaper.as_ref(),
                &bootstrap.artifact_root,
            )
            .await?;
        }
        Ok(Self {
            workers,
            admissions,
            migration_required,
            server,
            stop,
        })
    }

    pub async fn close(self, already_stopped: bool) -> Result<(), String> {
        let _ = self.stop.send(());
        if !already_stopped {
            let mut server = self.server;
            if tokio::time::timeout(Duration::from_secs(75), &mut server)
                .await
                .is_err()
            {
                server.abort();
                let _ = server.await;
                return Err("private listener did not drain".into());
            }
        }
        Ok(())
    }
}
