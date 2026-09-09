//! Dedicated delivery listener. Never merge this router into management or state HTTP.
mod handler;
mod rate;

use axum::{middleware, routing::any, Router};
use shaula_core::{
    operation_log::OperationLogReadPort,
    setup_info::{SetupInfoAuthorizer, SetupInfoConfig},
};
use std::{future::Future, io, net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, sync::Semaphore};

const ROUTE: &str = "/runner/v1/generations/{generation_id}/setup-info";

#[derive(Clone)]
struct App {
    authorizer: Arc<dyn SetupInfoAuthorizer>,
    logs: Arc<dyn OperationLogReadPort>,
    permits: Arc<Semaphore>,
    rate: Arc<rate::RateLimit>,
}

fn router(
    config: &SetupInfoConfig,
    authorizer: Arc<dyn SetupInfoAuthorizer>,
    logs: Arc<dyn OperationLogReadPort>,
) -> Router {
    Router::new()
        .route(ROUTE, any(handler::handle))
        .fallback(handler::unknown)
        .with_state(App {
            authorizer,
            logs,
            permits: Arc::new(Semaphore::new(config.max_concurrent_requests)),
            rate: Arc::new(rate::RateLimit::new(
                config.requests_per_second,
                config.burst,
            )),
        })
        .layer(middleware::from_fn(handler::safe_headers))
}

pub struct SetupInfoServer {
    listener: TcpListener,
    router: Router,
}

impl SetupInfoServer {
    pub async fn bind(
        config: SetupInfoConfig,
        authorizer: Arc<dyn SetupInfoAuthorizer>,
        logs: Arc<dyn OperationLogReadPort>,
    ) -> io::Result<Self> {
        config
            .validate()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let listener = TcpListener::bind(config.listen).await?;
        Ok(Self {
            listener,
            router: router(&config, authorizer, logs),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn serve(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> io::Result<()> {
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(shutdown)
            .await
    }
}

#[cfg(test)]
mod tests;
