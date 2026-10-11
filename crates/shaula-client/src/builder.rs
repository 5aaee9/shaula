use super::*;
#[derive(Debug, Clone)]
pub enum Credential {
    PersonalAccessToken(Secret),
    OidcAccessToken(Secret),
}
pub struct ClientBuilder {
    origin: String,
    credential: Credential,
    timeout: Duration,
    connect_timeout: Duration,
    upload_timeout: Duration,
    roots: Vec<reqwest::Certificate>,
    user_agent: String,
    loopback: bool,
}
impl ClientBuilder {
    pub(crate) fn new(origin: &str, credential: Credential) -> Self {
        Self {
            origin: origin.into(),
            credential,
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            upload_timeout: Duration::from_secs(300),
            roots: Vec::new(),
            user_agent: format!("shaula-client/{}", env!("CARGO_PKG_VERSION")),
            loopback: false,
        }
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }
    pub fn upload_timeout(mut self, timeout: Duration) -> Self {
        self.upload_timeout = timeout;
        self
    }
    pub fn add_root_certificate(mut self, root: reqwest::Certificate) -> Self {
        self.roots.push(root);
        self
    }
    pub fn user_agent(mut self, value: &str) -> Self {
        self.user_agent = value.into();
        self
    }
    pub fn allow_loopback_http(mut self, allow: bool) -> Self {
        self.loopback = allow;
        self
    }
    pub fn build(self) -> Result<Client, Error> {
        for timeout in [self.timeout, self.connect_timeout, self.upload_timeout] {
            if timeout < Duration::from_millis(100) || timeout > Duration::from_secs(3600) {
                return Err(Error::Invalid(
                    "timeout must be between 100 ms and one hour",
                ));
            }
        }
        let secret = match self.credential {
            Credential::PersonalAccessToken(secret)
                if secret.expose().starts_with("shaula_pat_v1_") =>
            {
                secret
            }
            Credential::OidcAccessToken(secret) if !secret.expose().starts_with("shaula_pat_") => {
                secret
            }
            _ => return Err(Error::Invalid("credential kind and input differ")),
        };
        let http = transport_builder(
            self.timeout,
            self.connect_timeout,
            self.user_agent,
            self.roots,
        );
        Client::assemble(
            &self.origin,
            secret,
            self.loopback,
            http,
            Error::Invalid("invalid transport configuration"),
            self.timeout,
            self.upload_timeout,
        )
    }
}

/// The one reqwest configuration every client uses: redirects disabled,
/// bounded timeouts, a fixed User-Agent and optional extra TLS roots.
pub(crate) fn transport_builder(
    timeout: Duration,
    connect_timeout: Duration,
    user_agent: impl Into<String>,
    roots: Vec<reqwest::Certificate>,
) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(connect_timeout)
        .user_agent(user_agent.into());
    for root in roots {
        builder = builder.add_root_certificate(root);
    }
    builder
}

/// Credential bytes must be non-empty and free of whitespace/control chars.
pub(crate) fn validate_credential(credential: &Secret) -> Result<(), Error> {
    if credential.expose().is_empty()
        || credential
            .expose()
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(Error::Invalid("invalid credential input"));
    }
    Ok(())
}
