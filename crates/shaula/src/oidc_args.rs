use clap::Args;
use shaula_core::secret::SecretString;
use shaula_daemon::config::AuthorizationGrant;
use shaula_http::oidc::{Grant, OidcConfig};

#[derive(Args)]
pub struct OidcArgs {
    /// Exact HTTPS issuer URL of the required OpenID Connect provider.
    #[arg(long, env = "SHAULA_OIDC_PROVIDER", hide_env_values = true)]
    oidc_provider: String,
    #[arg(long, env = "SHAULA_OIDC_CLIENT_ID", hide_env_values = true)]
    oidc_client_id: String,
    /// Fixed HTTPS origin of the Shaula browser entry point.
    #[arg(long, env = "SHAULA_OIDC_PUBLIC_URL", hide_env_values = true)]
    oidc_public_url: String,
    #[arg(long, env = "SHAULA_OIDC_API_AUDIENCE", hide_env_values = true)]
    oidc_api_audience: String,
    /// Optional PEM CA certificate for a private HTTPS identity provider.
    #[arg(long, env = "SHAULA_OIDC_CA_CERT", hide_env_values = true)]
    oidc_ca_cert: Option<std::path::PathBuf>,
}

impl OidcArgs {
    pub async fn initialize(
        self,
        grants: Vec<AuthorizationGrant>,
    ) -> Result<std::sync::Arc<shaula_http::oidc::Oidc>, String> {
        let roots = self
            .oidc_ca_cert
            .as_ref()
            .map(|path| {
                let pem = std::fs::read(path)
                    .map_err(|_| "oidc CA certificate cannot be read".to_owned())?;
                reqwest::Certificate::from_pem(&pem)
                    .map_err(|_| "oidc CA certificate invalid".to_owned())
            })
            .transpose()?
            .into_iter()
            .collect();
        shaula_http::oidc::Oidc::discover_with_roots(self.config(grants)?, roots)
            .await
            .map_err(|e| e.to_string())
    }

    pub fn config(self, grants: Vec<AuthorizationGrant>) -> Result<OidcConfig, String> {
        let secret = std::env::var("SHAULA_OIDC_CLIENT_SECRET")
            .map_err(|_| "SHAULA_OIDC_CLIENT_SECRET is required".to_owned())?;
        let grants = grants
            .into_iter()
            .map(|g| Grant {
                issuer: g.issuer,
                subject: g.subject,
                scopes: g.scopes,
            })
            .collect();
        OidcConfig::new(
            self.oidc_provider,
            self.oidc_client_id,
            SecretString::new(secret),
            self.oidc_public_url,
            self.oidc_api_audience,
            grants,
        )
        .map_err(|e| e.to_string())
    }
}
