use super::{session_refresh::RenewalState, tokens::BrowserBinding, AuthError, Authenticated};
use axum::http::{header, HeaderMap};
use oauth2::PkceCodeVerifier;
use sha2::{Digest, Sha256};
use shaula_core::secret::SecretString;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

pub(super) const SESSION: &str = "__Host-shaula-session";
pub(super) const TRANSACTION: &str = "__Host-shaula-login";
const LIMIT: usize = 4096;
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

pub(super) struct Transaction {
    pub nonce: String,
    pub binding: String,
    pub verifier: PkceCodeVerifier,
    pub target: String,
    pub expires: Instant,
    pub scopes: Vec<String>,
}

#[derive(Clone)]
pub(super) struct RefreshGrant {
    pub token: SecretString,
    pub binding: BrowserBinding,
    pub scopes: Vec<String>,
}

pub(super) struct Session {
    pub identity: Authenticated,
    pub expires: Instant,
    pub idle: Instant,
    pub refresh: Option<RefreshGrant>,
    pub renewal: RenewalState,
}

#[derive(Default)]
pub(super) struct Sessions {
    transactions: HashMap<String, Transaction>,
    pub(super) sessions: HashMap<String, Session>,
}

impl Sessions {
    pub(super) fn evict(&mut self) {
        let now = Instant::now();
        self.transactions.retain(|_, t| t.expires > now);
        self.sessions
            .retain(|_, s| s.refresh.is_some() || (s.expires > now && s.idle > now));
    }

    pub fn begin(&mut self, state: String, transaction: Transaction) -> Result<(), AuthError> {
        self.evict();
        if self.transactions.len() >= LIMIT {
            return Err(AuthError::Capacity);
        }
        self.transactions.insert(state, transaction);
        Ok(())
    }

    pub fn consume(&mut self, state: &str, binding: &str) -> Result<Transaction, AuthError> {
        self.evict();
        let transaction = self
            .transactions
            .remove(state)
            .ok_or(AuthError::Unauthorized)?;
        if !equal(&transaction.binding, binding) {
            return Err(AuthError::Unauthorized);
        }
        Ok(transaction)
    }

    pub fn create(
        &mut self,
        identity: Authenticated,
        exp: u64,
        old: Option<&str>,
    ) -> Result<(String, u64), AuthError> {
        let seconds = exp
            .saturating_sub(jsonwebtoken::get_current_timestamp())
            .min(3600);
        if seconds == 0 {
            return Err(AuthError::Unauthorized);
        }
        let id = self.insert(
            identity,
            Instant::now() + Duration::from_secs(seconds),
            None,
            old,
        )?;
        Ok((id, seconds))
    }

    pub(super) fn insert(
        &mut self,
        mut identity: Authenticated,
        expires: Instant,
        refresh: Option<RefreshGrant>,
        old: Option<&str>,
    ) -> Result<String, AuthError> {
        self.evict();
        if expires <= Instant::now() || expires > Instant::now() + Duration::from_secs(3600) {
            return Err(AuthError::Unauthorized);
        }
        if let Some(old) = old {
            self.sessions.remove(old);
        }
        if self.sessions.len() >= LIMIT {
            return Err(AuthError::Capacity);
        }
        let id = random();
        identity.csrf = Some(random());
        self.sessions.insert(
            id.clone(),
            Session {
                identity,
                expires,
                idle: Instant::now() + Duration::from_secs(900),
                refresh,
                renewal: RenewalState::Idle,
            },
        );
        Ok(id)
    }

    pub fn get(&mut self, id: &str) -> Result<Authenticated, AuthError> {
        self.evict();
        let session = self.sessions.get_mut(id).ok_or(AuthError::Unauthorized)?;
        if session.expires <= Instant::now() || session.idle <= Instant::now() {
            return Err(AuthError::Unauthorized);
        }
        session.idle = Instant::now() + Duration::from_secs(900);
        Ok(session.identity.clone())
    }

    pub fn remove(&mut self, id: &str) {
        self.sessions.remove(id);
    }

    /// Retention permits CSRF validation and local logout, never stale content access.
    pub(super) fn retained(&mut self, id: &str) -> Result<Authenticated, AuthError> {
        self.evict();
        self.sessions
            .get(id)
            .map(|session| session.identity.clone())
            .ok_or(AuthError::Unauthorized)
    }
}

pub(super) fn random() -> String {
    oauth2::CsrfToken::new_random().secret().clone()
}

pub(super) fn equal(a: &str, b: &str) -> bool {
    bool::from(Sha256::digest(a.as_bytes()).ct_eq(&Sha256::digest(b.as_bytes())))
}

pub(super) fn cookie_value(headers: &HeaderMap, name: &str) -> Result<Option<String>, AuthError> {
    let mut value = None;
    for header in headers.get_all(header::COOKIE) {
        for cookie in
            cookie::Cookie::split_parse(header.to_str().map_err(|_| AuthError::Unauthorized)?)
        {
            let cookie = cookie.map_err(|_| AuthError::Unauthorized)?;
            if cookie.name() == name {
                if value.is_some() || cookie.value().len() > 128 {
                    return Err(AuthError::Unauthorized);
                }
                value = Some(cookie.value().to_owned());
            }
        }
    }
    Ok(value)
}

pub(super) fn cookie(name: &'static str, value: String, seconds: i64) -> String {
    build_cookie(name, value, Some(seconds))
}

pub(super) fn session_cookie(value: String) -> String {
    build_cookie(SESSION, value, None)
}

fn build_cookie(name: &'static str, value: String, seconds: Option<i64>) -> String {
    let mut cookie = cookie::Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(cookie::SameSite::Lax);
    if let Some(seconds) = seconds {
        cookie = cookie.max_age(cookie::time::Duration::seconds(seconds));
    }
    cookie.build().to_string()
}
