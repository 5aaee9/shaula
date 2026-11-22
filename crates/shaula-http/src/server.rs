//! Listener helpers: mutation gate and graceful serving on a bound listener.

/// The composition root closes management mutations before quiescing effects.
/// Recovery-only startup keeps authenticated reads and login available.
pub fn with_mutation_gate(
    router: axum::Router,
    health: std::sync::Arc<dyn shaula_core::registry::HealthPort>,
    reason: &'static str,
) -> axum::Router {
    router.layer(axum::middleware::from_fn(move |request: axum::extract::Request, next: axum::middleware::Next| {
        let health = health.clone();
        async move {
            use axum::response::IntoResponse;
            if request.uri().path().starts_with("/api/") && !request.method().is_safe() && !health.ready().await {
                return crate::problem::problem(axum::http::StatusCode::SERVICE_UNAVAILABLE, reason, "management mutations are unavailable until lifecycle readiness is restored").into_response();
            }
            next.run(request).await
        }
    }))
}

pub async fn serve_bound(
    router: axum::Router,
    listener: tokio::net::TcpListener,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let mut shutdown = shutdown;
            let _ = shutdown.changed().await;
        })
        .await
        .map_err(|e| format!("server error: {e}"))
}
