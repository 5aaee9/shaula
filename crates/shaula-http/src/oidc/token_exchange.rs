//! Shared confidential-client exchange boundary; token payloads never enter errors.
use super::{AuthError, Oidc};
use oauth2::{
    basic::{
        BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
        BasicTokenType,
    },
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, EndpointNotSet, EndpointSet,
    PkceCodeVerifier, RedirectUrl, RefreshToken, StandardRevocableToken, StandardTokenResponse,
    TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use shaula_core::secret::SecretString;
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "token_exchange_tests.rs"]
mod tests;

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Extra {
    pub id_token: Option<String>,
}
impl std::fmt::Debug for Extra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl oauth2::ExtraTokenFields for Extra {}
pub(super) type Tokens = StandardTokenResponse<Extra, BasicTokenType>;
type LoginClient<A = EndpointNotSet, T = EndpointNotSet> = Client<
    BasicErrorResponse,
    Tokens,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    A,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    T,
>;

impl Oidc {
    pub(super) async fn client(&self) -> Result<LoginClient<EndpointSet, EndpointSet>, AuthError> {
        let mut provider = self.provider.lock().await;
        let refresh = provider.failed;
        provider.current(&self.http, &self.config, refresh).await?;
        Ok(
            LoginClient::new(ClientId::new(self.config.client_id.clone()))
                .set_client_secret(ClientSecret::new(self.config.secret.expose().to_owned()))
                .set_auth_uri(
                    AuthUrl::new(provider.metadata.authorization_endpoint.clone())
                        .map_err(|_| AuthError::Provider)?,
                )
                .set_token_uri(
                    TokenUrl::new(provider.metadata.token_endpoint.clone())
                        .map_err(|_| AuthError::Provider)?,
                )
                .set_redirect_uri(
                    RedirectUrl::new(format!("{}/auth/oidc/callback", self.config.origin))
                        .map_err(|_| AuthError::Configuration("redirect URL"))?,
                ),
        )
    }

    pub(super) async fn exchange_code(
        &self,
        code: AuthorizationCode,
        verifier: PkceCodeVerifier,
    ) -> Result<Tokens, AuthError> {
        let http = self.http.clone();
        self.client()
            .await?
            .exchange_code(code)
            .set_pkce_verifier(verifier)
            .request_async(&move |request| transport(http.clone(), request))
            .await
            .map_err(exchange_error)
    }

    pub(super) async fn exchange_refresh(&self, token: &SecretString) -> Result<Tokens, AuthError> {
        let http = self.http.clone();
        self.client()
            .await?
            .exchange_refresh_token(&RefreshToken::new(token.expose().to_owned()))
            .request_async(&move |request| transport(http.clone(), request))
            .await
            .map_err(exchange_error)
    }
}

async fn transport(
    http: reqwest::Client,
    request: oauth2::HttpRequest,
) -> Result<oauth2::HttpResponse, AuthError> {
    let response = http
        .request(request.method().clone(), request.uri().to_string())
        .headers(request.headers().clone())
        .body(request.body().clone())
        .send()
        .await
        .map_err(|_| AuthError::Provider)?;
    if response.status().is_server_error()
        || response.status() == http::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(AuthError::Provider);
    }
    let oversized = if response.status().is_success() {
        AuthError::Unauthorized
    } else {
        AuthError::Provider
    };
    super::provider::bounded_body(response, oversized).await
}

fn exchange_error(error: oauth2::RequestTokenError<AuthError, BasicErrorResponse>) -> AuthError {
    match error {
        oauth2::RequestTokenError::ServerResponse(error)
            if *error.error() == oauth2::basic::BasicErrorResponseType::InvalidGrant =>
        {
            AuthError::Unauthorized
        }
        oauth2::RequestTokenError::Request(error) => error,
        oauth2::RequestTokenError::Parse(..) | oauth2::RequestTokenError::Other(_) => {
            AuthError::Unauthorized
        }
        _ => AuthError::Provider,
    }
}

/// A token response is successful only with fresh, bounded evidence.
pub(super) fn expiry(
    tokens: &Tokens,
    id_expiry: Option<u64>,
    received: Instant,
) -> Result<Instant, AuthError> {
    if tokens.access_token().secret().is_empty()
        || *tokens.token_type() != BasicTokenType::Bearer
        || tokens
            .refresh_token()
            .is_some_and(|token| token.secret().is_empty())
    {
        return Err(AuthError::Unauthorized);
    }
    let lifetime = tokens.expires_in();
    if lifetime.is_none() && id_expiry.is_none() {
        return Err(AuthError::Unauthorized);
    }
    let now = Instant::now();
    let hour = Duration::from_secs(3600);
    let mut expires = received + hour;
    if let Some(lifetime) = lifetime {
        expires = expires.min(received + lifetime.min(hour));
    }
    if let Some(id_expiry) = id_expiry {
        let remaining =
            Duration::from_secs(id_expiry.saturating_sub(jsonwebtoken::get_current_timestamp()));
        expires = expires.min(now + remaining.min(hour));
    }
    if expires <= now {
        return Err(AuthError::Unauthorized);
    }
    Ok(expires)
}
