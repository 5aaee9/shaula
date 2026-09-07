use super::{
    sessions::{self, Transaction, SESSION, TRANSACTION},
    AuthError, Authenticated, Oidc,
};
use crate::router::AppState;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use oauth2::{
    basic::{
        BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
        BasicTokenType,
    },
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, PkceCodeChallenge, RedirectUrl, Scope, StandardRevocableToken,
    StandardTokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Deserialize, Serialize)]
struct Extra {
    id_token: String,
}
impl std::fmt::Debug for Extra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl oauth2::ExtraTokenFields for Extra {}
type LoginClient<A = EndpointNotSet, T = EndpointNotSet> = Client<
    BasicErrorResponse,
    StandardTokenResponse<Extra, BasicTokenType>,
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
    async fn client(&self) -> Result<LoginClient<EndpointSet, EndpointSet>, AuthError> {
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
}

#[derive(Deserialize, Default)]
pub(crate) struct LoginQuery {
    return_to: Option<String>,
}

pub(crate) async fn login(
    State(state): State<AppState>,
    query: Result<Query<LoginQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, AuthError> {
    let Query(query) = query.map_err(|_| AuthError::Unauthorized)?;
    let oidc = &state.oidc;
    let client = oidc.client().await?;
    let profile = oidc
        .provider
        .lock()
        .await
        .metadata
        .scopes_supported
        .as_ref()
        .is_some_and(|scopes| scopes.iter().any(|scope| scope == "profile"));
    let nonce = sessions::random();
    let binding = sessions::random();
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut authorization = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("openid".into()))
        .add_extra_param("nonce", &nonce)
        .set_pkce_challenge(challenge);
    if profile {
        authorization = authorization.add_scope(Scope::new("profile".into()));
    }
    let (url, csrf) = authorization.url();
    let transaction = Transaction {
        nonce,
        binding: binding.clone(),
        verifier,
        target: return_target(query.return_to.as_deref()),
        expires: Instant::now() + Duration::from_secs(600),
    };
    oidc.sessions
        .lock()
        .await
        .begin(csrf.secret().clone(), transaction)?;
    let mut response = redirect(url.as_str())?;
    set_cookie(&mut response, sessions::cookie(TRANSACTION, binding, 600))?;
    Ok(response)
}

#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub(crate) async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<CallbackQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, AuthError> {
    let Query(query) = query.map_err(|_| AuthError::Unauthorized)?;
    let binding = sessions::cookie_value(&headers, TRANSACTION)?.ok_or(AuthError::Unauthorized)?;
    let transaction = state.oidc.sessions.lock().await.consume(
        query.state.as_deref().ok_or(AuthError::Unauthorized)?,
        &binding,
    )?;
    if query.error.is_some() {
        return Err(AuthError::Unauthorized);
    }
    let code = query
        .code
        .filter(|c| !c.is_empty() && c.len() <= 4096)
        .ok_or(AuthError::Unauthorized)?;
    let oidc = &state.oidc;
    let _permit = oidc
        .exchanges
        .try_acquire()
        .map_err(|_| AuthError::Capacity)?;
    let client = oidc.client().await?;
    let transport = |request: oauth2::HttpRequest| {
        let http = oidc.http.clone();
        async move {
            let response = super::provider::bounded(
                http.request(request.method().clone(), request.uri().to_string())
                    .headers(request.headers().clone())
                    .body(request.body().clone()),
            )
            .await?;
            if response.status().is_server_error()
                || response.status() == StatusCode::TOO_MANY_REQUESTS
            {
                return Err(AuthError::Provider);
            }
            Ok(response)
        }
    };
    let tokens = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(transaction.verifier)
        .request_async(&transport)
        .await;
    let tokens = match tokens {
        Ok(tokens) => tokens,
        Err(oauth2::RequestTokenError::ServerResponse(error))
            if *error.error() == oauth2::basic::BasicErrorResponseType::InvalidGrant =>
        {
            return Err(AuthError::Unauthorized)
        }
        Err(_) => {
            oidc.login_available
                .store(false, std::sync::atomic::Ordering::Relaxed);
            return Err(AuthError::Provider);
        }
    };
    let (identity, expiry) = oidc
        .browser_identity(&tokens.extra_fields().id_token, &transaction.nonce)
        .await?;
    oidc.login_available
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let old = sessions::cookie_value(&headers, SESSION)?;
    let (id, seconds) = oidc
        .sessions
        .lock()
        .await
        .create(identity, expiry, old.as_deref())?;
    let mut response = redirect(&transaction.target)?;
    set_cookie(&mut response, sessions::cookie(SESSION, id, seconds as i64))?;
    set_cookie(
        &mut response,
        sessions::cookie(TRANSACTION, String::new(), 0),
    )?;
    Ok(response)
}

pub(crate) async fn logout(
    State(state): State<AppState>,
    _: Authenticated,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let id = sessions::cookie_value(&headers, SESSION)?.ok_or(AuthError::Unauthorized)?;
    state.oidc.sessions.lock().await.remove(&id);
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(&mut response, sessions::cookie(SESSION, String::new(), 0))?;
    Ok(response)
}

pub(crate) fn document(path: &str) -> bool {
    matches!(path, "/" | "/fleets" | "/templates" | "/auth" | "/changes")
        || path.strip_prefix("/fleets/").is_some_and(|key| {
            !matches!(key, "." | "..") && shaula_core::fleet::FleetKey::new(key).is_ok()
        })
}

pub(super) fn return_target(raw: Option<&str>) -> String {
    let Some(raw) = raw.filter(|r| {
        r.len() <= 2048 && !r.contains(['\\', '#']) && !r.chars().any(char::is_control)
    }) else {
        return "/fleets".into();
    };
    let (path, query) = raw.split_once('?').map_or((raw, ""), |(p, q)| (p, q));
    if !document(path) {
        return "/fleets".into();
    }
    let params: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
    if params.iter().any(|(key, value)| {
        !matches!(key.as_ref(), "key" | "id" | "type")
            || value.len() > 256
            || value.chars().any(char::is_control)
            || value.contains(['\\', '/', '%'])
    }) {
        return "/fleets".into();
    }
    raw.to_owned()
}

pub(super) fn redirect(target: &str) -> Result<Response, AuthError> {
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            HeaderValue::from_str(target).map_err(|_| AuthError::Unauthorized)?,
        )],
    )
        .into_response())
}

fn set_cookie(response: &mut Response, value: String) -> Result<(), AuthError> {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&value).map_err(|_| AuthError::Unauthorized)?,
    );
    Ok(())
}
