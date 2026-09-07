//! Mandatory server-side OIDC boundary. No header assertion or anonymous mode.
mod config;
mod guard;
mod login;
mod provider;
mod sessions;
#[cfg(test)]
#[path = "../../tests/support/mod.rs"]
mod support;
mod tokens;

pub use config::{Grant, OidcConfig};
pub(crate) use guard::guard;
pub(crate) use login::document;
pub(crate) use login::{callback, login, logout};

use axum::{
    extract::FromRequestParts,
    http::request::Parts,
    response::{IntoResponse, Response},
};
use shaula_core::registry::Actor;
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum AuthError {
    #[error("oidc configuration invalid: {0}")]
    Configuration(&'static str),
    #[error("oidc provider unavailable or incompatible")]
    Provider,
    #[error("authentication required")]
    Unauthorized,
    #[error("request origin or csrf validation failed")]
    Csrf,
    #[error("authentication capacity exhausted")]
    Capacity,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        use axum::http::{header, StatusCode};
        let status = match self {
            Self::Csrf => StatusCode::FORBIDDEN,
            Self::Provider | Self::Capacity | Self::Configuration(_) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
        };
        let mut response =
            crate::problem::problem(status, "AuthenticationFailed", self.to_string())
                .into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}

#[derive(Clone)]
pub struct Authenticated {
    pub actor: Actor,
    pub name: String,
    pub(crate) csrf: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for Authenticated {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or_else(|| AuthError::Unauthorized.into_response())
    }
}

pub struct Oidc {
    config: OidcConfig,
    http: reqwest::Client,
    provider: Mutex<provider::ProviderCache>,
    sessions: Mutex<sessions::Sessions>,
    exchanges: tokio::sync::Semaphore,
    login_available: std::sync::atomic::AtomicBool,
}

impl Oidc {
    pub async fn discover(config: OidcConfig) -> Result<Arc<Self>, AuthError> {
        Self::discover_with_roots(config, Vec::new()).await
    }

    /// Additional CA roots permit private identity providers without disabling TLS verification.
    pub async fn discover_with_roots(
        config: OidcConfig,
        roots: Vec<reqwest::Certificate>,
    ) -> Result<Arc<Self>, AuthError> {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5));
        for root in roots {
            builder = builder.add_root_certificate(root);
        }
        let http = builder
            .build()
            .map_err(|_| AuthError::Configuration("HTTP client"))?;
        let provider = provider::ProviderCache::load(&http, &config).await?;
        Ok(Arc::new(Self {
            config,
            http,
            provider: Mutex::new(provider),
            sessions: Mutex::new(sessions::Sessions::default()),
            exchanges: tokio::sync::Semaphore::new(16),
            login_available: std::sync::atomic::AtomicBool::new(true),
        }))
    }

    pub async fn ready(&self) -> bool {
        let mut provider = self.provider.lock().await;
        let refresh = provider.failed;
        provider
            .current(&self.http, &self.config, refresh)
            .await
            .is_ok()
            && !provider.failed
            && self
                .login_available
                .load(std::sync::atomic::Ordering::Relaxed)
    }
}
