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
        let mut client = Client::build(&self.origin, secret, self.loopback, Vec::new())?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout)
            .connect_timeout(self.connect_timeout)
            .user_agent(self.user_agent);
        for root in self.roots {
            builder = builder.add_root_certificate(root);
        }
        client.http = builder
            .build()
            .map_err(|_| Error::Invalid("invalid transport configuration"))?;
        client.timeout = self.timeout;
        client.upload_timeout = self.upload_timeout;
        Ok(client)
    }
}
