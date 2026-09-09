//! Composition of optional log delivery and durable operation diagnostics.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shaula_core::ports::Clock;
use shaula_core::setup_info::SetupInfoIssuer;
use shaula_daemon::bootstrap::ValidatedBootstrap;
use shaula_http::setup_info::SetupInfoServer;
use shaula_store::operation_logs::OperationLogArchive;
use shaula_store::setup_info::SetupInfoCapabilityRegistry;
use shaula_store::Store;

pub(crate) struct Diagnostics {
    pub logs: Option<Arc<OperationLogArchive>>,
    pub issuer: Option<Arc<dyn SetupInfoIssuer>>,
    server: Option<SetupInfoServer>,
    store: Store,
    metadata_retention_days: u32,
    delivery_available: Arc<AtomicBool>,
}

struct AvailableIssuer {
    inner: Arc<dyn SetupInfoIssuer>,
    available: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl SetupInfoIssuer for AvailableIssuer {
    async fn issue(
        &self,
        generation_id: &str,
        now: i64,
    ) -> shaula_core::CoreResult<shaula_core::template::SetupInfoDescriptor> {
        if self.available.load(Ordering::Acquire) {
            self.inner.issue(generation_id, now).await
        } else {
            Ok(shaula_core::template::SetupInfoDescriptor::Disabled)
        }
    }
}

impl Diagnostics {
    pub async fn new(store: Store, bootstrap: &ValidatedBootstrap) -> Self {
        let logs = match OperationLogArchive::open(
            store.clone(),
            bootstrap.data_dir.join("logs"),
            bootstrap.operation_logs.clone(),
        )
        .await
        {
            Ok(archive) => Some(Arc::new(archive)),
            Err(_) => {
                tracing::warn!(
                    "operation log archive unavailable; execution continues without capture"
                );
                None
            }
        };
        let mut diagnostics = Self {
            logs,
            issuer: None,
            server: None,
            store: store.clone(),
            metadata_retention_days: bootstrap.operation_logs.metadata_retention_days,
            delivery_available: Arc::new(AtomicBool::new(false)),
        };
        if let (Some(config), Some(logs)) = (&bootstrap.setup_info, &diagnostics.logs) {
            let registry = match SetupInfoCapabilityRegistry::new(store, config.clone()) {
                Ok(registry) => Arc::new(registry),
                Err(_) => {
                    tracing::warn!("setup log capability registry unavailable");
                    return diagnostics;
                }
            };
            match SetupInfoServer::bind(config.clone(), registry.clone(), logs.clone()).await {
                Ok(server) => {
                    diagnostics
                        .delivery_available
                        .store(true, Ordering::Release);
                    diagnostics.issuer = Some(Arc::new(AvailableIssuer {
                        inner: registry,
                        available: diagnostics.delivery_available.clone(),
                    }));
                    diagnostics.server = Some(server);
                }
                Err(_) => tracing::warn!(
                    "setup log listener unavailable; runners continue without setup log delivery"
                ),
            }
        }
        diagnostics
    }

    pub async fn run(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut server_shutdown = shutdown.clone();
        let mut servers = tokio::task::JoinSet::new();
        if let Some(server) = self.server {
            servers.spawn(async move {
                server
                    .serve(async move {
                        if !*server_shutdown.borrow() {
                            let _ = server_shutdown.changed().await;
                        }
                    })
                    .await
            });
        }
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                _ = servers.join_next(), if !servers.is_empty() => {
                    self.delivery_available.store(false, Ordering::Release);
                    if !*shutdown.borrow() { tracing::warn!("setup log listener stopped unexpectedly"); }
                },
                _ = ticker.tick() => {
                    if let Some(logs) = &self.logs {
                        match tokio::time::timeout(Duration::from_secs(5), logs.maintenance()).await {
                            Ok(Ok(())) => {},
                            _ => tracing::warn!("operation log maintenance incomplete"),
                        }
                    }
                    let cutoff = crate::SystemClock.now_unix_ms().saturating_sub(i64::from(self.metadata_retention_days) * 86_400_000);
                    if !matches!(tokio::time::timeout(Duration::from_secs(5), self.store.prune_job_history(cutoff)).await, Ok(Ok(()))) {
                        tracing::warn!("job history maintenance incomplete");
                    }
                },
                _ = shutdown.changed() => break,
            }
        }
        self.delivery_available.store(false, Ordering::Release);
        if !servers.is_empty()
            && tokio::time::timeout(Duration::from_secs(15), servers.join_next())
                .await
                .is_err()
        {
            servers.shutdown().await;
        }
    }
}
