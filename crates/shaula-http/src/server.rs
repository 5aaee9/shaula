//! Listener configuration: loopback-only binding fails closed on any
//! non-loopback address (ADR-0013). HTTPS terminates at the deployment proxy.

use shaula_core::net::verify_loopback;

pub use shaula_core::net::verify_loopback as loopback_policy;

/// Static server configuration validated before the API becomes ready.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub body_limit: usize,
}

impl ServerConfig {
    pub fn new(host: impl Into<String>, port: u16, body_limit: usize) -> Result<Self, String> {
        let host = host.into();
        verify_loopback(&host)?;
        Ok(Self {
            host,
            port,
            body_limit,
        })
    }

    pub fn listen_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Binds and serves a ready router on a validated loopback address. The
/// caller owns shutdown: a `true` on the watch channel begins the graceful
/// drain. A bind failure returns Err — daemon-fatal, never a clean stop.
pub async fn serve(
    router: axum::Router,
    addr: String,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bind {addr} failed: {e}"))?;
    // Announce readiness only AFTER the bind actually succeeded — an
    // early print would advertise a listener that is about to fail.
    println!("shaula listening on {addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let mut shutdown = shutdown;
            let _ = shutdown.changed().await;
        })
        .await
        .map_err(|e| format!("server error: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn non_loopback_bind_fails_closed() {
        assert!(ServerConfig::new("127.0.0.1", 8080, 1024).is_ok());
        assert!(ServerConfig::new("0.0.0.0", 8080, 1024).is_err());
    }

    #[test]
    fn server_config_formats_address() {
        let config = ServerConfig::new("127.0.0.1", 8080, 1024).unwrap();
        assert_eq!(config.listen_addr(), "127.0.0.1:8080");
    }
}
