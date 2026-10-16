//! Composition of the optional loopback Prometheus listener
//! (`observability.metrics_listen`), separate from management HTTP.

use std::sync::Arc;

use shaula_daemon::service::ControlPlane;
use shaula_http::metrics::Scrape;
use shaula_store::Store;

type Server = tokio::task::JoinHandle<Result<(), String>>;

/// Binds the metrics listener when configured and serves it until
/// shutdown. A bind failure fails startup like the management listener.
pub(crate) async fn start(
    listen: Option<std::net::SocketAddr>,
    store: Store,
    control: Arc<ControlPlane>,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<Option<Server>, String> {
    let Some(listen) = listen else {
        return Ok(None);
    };
    shaula_http::server::loopback_policy(&listen.ip().to_string())?;
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|_| "metrics listener bind failed")?;
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64().floor())
        .unwrap_or_default();
    let scrape = Arc::new(Scrape {
        ledger: Arc::new(store),
        fleets: control.clone(),
        health: control,
        started_at,
    });
    println!("shaula metrics listening on {listen}");
    Ok(Some(tokio::spawn(shaula_http::server::serve_bound(
        shaula_http::metrics::router(scrape),
        listener,
        shutdown,
    ))))
}

/// Awaits the drained metrics server after shutdown was signalled; a
/// scrape connection that outlives the bound is aborted.
pub(crate) async fn stop(server: Option<Server>) {
    let Some(mut server) = server else { return };
    if tokio::time::timeout(std::time::Duration::from_secs(5), &mut server)
        .await
        .is_err()
    {
        server.abort();
        let _ = server.await;
    }
}
