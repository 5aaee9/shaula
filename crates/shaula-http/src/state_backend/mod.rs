//! A dedicated private loopback listener, never merged into the OIDC router.
//! No access log/tracing layer: state, query lock IDs and Authorization are
//! credential-grade. Routes are intentionally not exposed as a public router.

mod auth;
mod handler;

use std::{future::Future, io, net::SocketAddr, sync::Arc, time::Duration};

use axum::{middleware, routing::any, Router};
use shaula_core::state_backend::StateBackend;
use tokio::{net::TcpListener, sync::Semaphore};

pub const STATE_ROUTE: &str = "/internal/v1/generations/{generation_id}/state";
const MAX_REQUESTS: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct App {
    backend: Arc<dyn StateBackend>,
    permits: Arc<Semaphore>,
}

fn router(backend: Arc<dyn StateBackend>) -> Router {
    Router::new()
        .route(STATE_ROUTE, any(handler::handle))
        .fallback(handler::unknown)
        .with_state(App {
            backend,
            permits: Arc::new(Semaphore::new(MAX_REQUESTS)),
        })
        .layer(middleware::from_fn(handler::no_store))
}

/// Ownership of the bound socket makes readiness explicit. The caller must
/// keep this alive until workers finish state writes and UNLOCK during shutdown.
/// It does not admit workers or change the management authentication policy.
pub struct StateServer {
    listener: TcpListener,
    router: Router,
}

impl StateServer {
    pub async fn bind(address: SocketAddr, backend: Arc<dyn StateBackend>) -> io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "private backend requires loopback",
            ));
        }
        let listener = TcpListener::bind(address).await?;
        Ok(Self {
            listener,
            router: router(backend),
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
