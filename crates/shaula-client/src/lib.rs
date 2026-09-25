//! Async API-only client. Mutation attempts are immutable and explicitly replayed.
mod builder;
mod problem;
mod resources;
mod tokens;
mod transport;
mod typed;
mod typed_registry;
mod wait;
pub use builder::{ClientBuilder, Credential};
use reqwest::{Method, Url};
pub use resources::*;
pub use shaula_api_types as types;
use std::time::Duration;
pub use typed::TypedResource;
pub use types::Secret;
pub use wait::ChangeKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceVersion(String);
impl ResourceVersion {
    pub fn parse(raw: &str) -> Result<Self, Error> {
        if raw.len() < 3
            || !raw.starts_with('"')
            || !raw.ends_with('"')
            || raw[1..raw.len() - 1]
                .bytes()
                .any(|b| b <= 0x20 || b >= 0x7f || b == b'"')
        {
            return Err(Error::Invalid("a strong resource version is required"));
        }
        Ok(Self(raw.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone)]
pub enum WritePrecondition {
    CreateOnly,
    Match(ResourceVersion),
}
#[derive(Debug, Clone)]
pub struct MutationOptions {
    pub precondition: WritePrecondition,
    pub idempotency_key: String,
}
impl MutationOptions {
    pub fn create() -> Self {
        Self {
            precondition: WritePrecondition::CreateOnly,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        }
    }
    pub fn update(version: ResourceVersion) -> Self {
        Self {
            precondition: WritePrecondition::Match(version),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        }
    }
}
#[derive(Debug)]
pub struct Resource<T> {
    pub data: T,
    pub version: Option<ResourceVersion>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("HTTP {status}: {code}")]
    Http {
        status: u16,
        code: String,
        retry_after_seconds: Option<u64>,
        request_id: Option<String>,
    },
    #[error("transport unavailable; mutation outcome may be uncertain")]
    Transport,
    #[error("server returned an invalid response")]
    Protocol,
    #[error("operation is unsupported by this server")]
    Unsupported,
    #[error("tracking ended before convergence: {0}")]
    Tracking(String),
}

#[derive(Clone)]
pub struct Client {
    origin: Url,
    http: reqwest::Client,
    credential: Secret,
    timeout: Duration,
    upload_timeout: Duration,
}
impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
impl Client {
    pub fn builder(origin: &str, credential: Credential) -> ClientBuilder {
        ClientBuilder::new(origin, credential)
    }
    pub fn new(origin: &str, credential: Secret) -> Result<Self, Error> {
        Self::build(origin, credential, false, vec![])
    }
    /// Explicitly opt into loopback HTTP for a local tunnel/test; remote plaintext is rejected.
    pub fn loopback(origin: &str, credential: Secret) -> Result<Self, Error> {
        Self::build(origin, credential, true, vec![])
    }
    pub fn with_roots(
        origin: &str,
        credential: Secret,
        roots: Vec<reqwest::Certificate>,
    ) -> Result<Self, Error> {
        Self::build(origin, credential, false, roots)
    }
    fn build(
        origin: &str,
        credential: Secret,
        loopback: bool,
        roots: Vec<reqwest::Certificate>,
    ) -> Result<Self, Error> {
        let url = Url::parse(origin).map_err(|_| Error::Invalid("invalid server origin"))?;
        let local = url
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if (url.scheme() != "https" && !(loopback && local && url.scheme() == "http"))
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || origin.contains('\\')
            || origin.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(Error::Invalid(
                "server must be an HTTPS origin without userinfo, path, query or fragment",
            ));
        }
        if credential.expose().is_empty()
            || credential
                .expose()
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(Error::Invalid("invalid credential input"));
        }
        let timeout = Duration::from_secs(30);
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("shaula-client/", env!("CARGO_PKG_VERSION")));
        for root in roots {
            builder = builder.add_root_certificate(root);
        }
        Ok(Self {
            origin: url,
            http: builder.build().map_err(|_| Error::Transport)?,
            credential,
            timeout,
            upload_timeout: Duration::from_secs(300),
        })
    }
    pub fn origin(&self) -> &str {
        self.origin.as_str()
    }
    /// Explicit secret handoff for a caller's secure credential store.
    pub fn credential_for_storage(&self) -> &Secret {
        &self.credential
    }
    /// Retain the origin, TLS roots, redirect policy and timeouts during rotation.
    pub fn with_credential(&self, credential: Secret) -> Result<Self, Error> {
        if credential.expose().is_empty()
            || credential
                .expose()
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(Error::Invalid("invalid credential input"));
        }
        Ok(Self {
            credential,
            ..self.clone()
        })
    }
    pub async fn session(&self) -> Result<Resource<types::Session>, Error> {
        self.read(&["api", "v1", "session"], &[]).await
    }
    pub async fn live(&self) -> Result<bool, Error> {
        self.health("livez").await
    }
    pub async fn ready(&self) -> Result<bool, Error> {
        self.health("readyz").await
    }
}

/// Bytes, identity, origin, method, path, key and precondition cannot change on retry.
pub struct MutationAttempt {
    origin: String,
    identity: AttemptIdentity,
    method: Method,
    path: Vec<String>,
    body: Secret,
    precondition: Option<WritePrecondition>,
    key: String,
    content_type: &'static str,
}
enum AttemptIdentity {
    Principal(types::Principal),
    // Old servers lack a stable principal projection. Bind to the exact
    // successfully authenticated OIDC credential instead of guessing from a name.
    LegacyOidc(Secret),
}
impl std::fmt::Debug for MutationAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationAttempt")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}
impl MutationAttempt {
    pub fn idempotency_key(&self) -> &str {
        &self.key
    }
}
