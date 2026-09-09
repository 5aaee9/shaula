use super::{
    sessions::{self, RefreshGrant, Transaction, SESSION, TRANSACTION},
    token_exchange, AuthError, Authenticated,
};
use crate::router::AppState;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use oauth2::{AuthorizationCode, CsrfToken, PkceCodeChallenge, Scope, TokenResponse};
use serde::Deserialize;
use shaula_core::secret::SecretString;
use std::time::{Duration, Instant};

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
    let mut scopes = vec!["openid".to_owned()];
    if profile {
        authorization = authorization.add_scope(Scope::new("profile".into()));
        scopes.push("profile".to_owned());
    }
    let (url, csrf) = authorization.url();
    let transaction = Transaction {
        nonce,
        binding: binding.clone(),
        verifier,
        target: return_target(query.return_to.as_deref()),
        expires: Instant::now() + Duration::from_secs(600),
        scopes,
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
    let tokens = oidc
        .exchange_code(AuthorizationCode::new(code), transaction.verifier)
        .await;
    let tokens = match tokens {
        Ok(tokens) => tokens,
        Err(AuthError::Unauthorized) => return Err(AuthError::Unauthorized),
        Err(_) => {
            oidc.login_available
                .store(false, std::sync::atomic::Ordering::Relaxed);
            return Err(AuthError::Provider);
        }
    };
    let received = Instant::now();
    let login = oidc
        .browser_login(
            tokens
                .extra_fields()
                .id_token
                .as_deref()
                .ok_or(AuthError::Unauthorized)?,
            &transaction.nonce,
        )
        .await?;
    oidc.login_available
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let old = sessions::cookie_value(&headers, SESSION)?;
    if tokens
        .refresh_token()
        .is_some_and(|token| token.secret().is_empty())
    {
        return Err(AuthError::Unauthorized);
    }
    let refresh = tokens.refresh_token();
    let session_cookie = if let Some(refresh) = refresh {
        let expires = token_exchange::expiry(&tokens, Some(login.expiry), received)?;
        let scopes = tokens.scopes().map_or(transaction.scopes, |scopes| {
            scopes
                .iter()
                .map(|scope| scope.as_str().to_owned())
                .collect()
        });
        if !scopes.iter().any(|scope| scope == "openid") {
            return Err(AuthError::Unauthorized);
        }
        let grant = RefreshGrant {
            token: SecretString::new(refresh.secret().as_str()),
            binding: login.binding,
            scopes,
        };
        let id = oidc.sessions.lock().await.insert(
            login.identity,
            expires,
            Some(grant),
            old.as_deref(),
        )?;
        sessions::session_cookie(id)
    } else {
        let (id, seconds) =
            oidc.sessions
                .lock()
                .await
                .create(login.identity, login.expiry, old.as_deref())?;
        sessions::cookie(
            SESSION,
            id,
            i64::try_from(seconds).map_err(|_| AuthError::Unauthorized)?,
        )
    };
    let mut response = redirect(&transaction.target)?;
    set_cookie(&mut response, session_cookie)?;
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
    matches!(
        path,
        "/" | "/fleets" | "/templates" | "/templates/new" | "/auth" | "/changes"
    ) || path.strip_prefix("/fleets/").is_some_and(|key| {
        !matches!(key, "." | "..") && shaula_core::fleet::FleetKey::new(key).is_ok()
    }) || path
        .strip_prefix("/templates/")
        .and_then(|path| path.strip_suffix("/revisions/new"))
        .is_some_and(|key| {
            !matches!(key, "." | "..")
                && shaula_core::template::TemplateProfileKey::new(key).is_ok()
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
