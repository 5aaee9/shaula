use super::{AuthError, Authenticated, Oidc};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use shaula_core::registry::Actor;

#[derive(Deserialize)]
pub(super) struct Claims {
    pub iss: String,
    pub sub: String,
    pub aud: Audience,
    pub exp: u64,
    pub iat: u64,
    pub nbf: Option<u64>,
    pub azp: Option<String>,
    pub nonce: Option<String>,
    pub name: Option<String>,
    pub client_id: Option<String>,
    pub jti: Option<String>,
    pub scope: Option<String>,
    pub auth_time: Option<u64>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn values(&self) -> std::collections::BTreeSet<&str> {
        match self {
            Self::One(value) => [value.as_str()].into_iter().collect(),
            Self::Many(values) => values.iter().map(String::as_str).collect(),
        }
    }
    fn matches(&self, expected: &str) -> bool {
        match self {
            Self::One(s) => s == expected,
            Self::Many(s) => s.iter().any(|s| s == expected),
        }
    }
    fn multiple(&self) -> bool {
        matches!(self, Self::Many(s) if s.len() > 1)
    }
}

#[derive(Clone)]
pub(super) struct BrowserBinding {
    pub subject: String,
    pub audience: Audience,
    pub nonce: String,
    pub auth_time: Option<u64>,
}

pub(super) struct BrowserLogin {
    pub identity: Authenticated,
    pub expiry: u64,
    pub binding: BrowserBinding,
}

impl Oidc {
    #[cfg(test)]
    pub(super) async fn browser_identity(
        &self,
        token: &str,
        nonce: &str,
    ) -> Result<(Authenticated, u64), AuthError> {
        let login = self.browser_login(token, nonce).await?;
        Ok((login.identity, login.expiry))
    }

    pub(super) async fn browser_login(
        &self,
        token: &str,
        nonce: &str,
    ) -> Result<BrowserLogin, AuthError> {
        let claims = self.claims(token, false).await?;
        if !claims
            .nonce
            .as_deref()
            .is_some_and(|n| super::sessions::equal(n, nonce))
        {
            return Err(AuthError::Unauthorized);
        }
        Ok(BrowserLogin {
            identity: self.identity(&claims, false)?,
            expiry: claims.exp,
            binding: BrowserBinding {
                subject: claims.sub,
                audience: claims.aud,
                nonce: nonce.to_owned(),
                auth_time: claims.auth_time,
            },
        })
    }

    pub(super) async fn refreshed_identity(
        &self,
        token: Option<&str>,
        binding: &BrowserBinding,
        original: &Authenticated,
    ) -> Result<(Authenticated, Option<u64>), AuthError> {
        let Some(token) = token else {
            let mut identity = original.clone();
            identity.actor.scopes = self.config.scopes(&binding.subject);
            return Ok((identity, None));
        };
        let claims = self.claims(token, false).await?;
        if claims.sub != binding.subject
            || claims.aud.values() != binding.audience.values()
            || claims
                .nonce
                .as_deref()
                .is_some_and(|nonce| !super::sessions::equal(nonce, &binding.nonce))
            || claims
                .auth_time
                .is_some_and(|time| Some(time) != binding.auth_time)
        {
            return Err(AuthError::Unauthorized);
        }
        Ok((self.identity(&claims, false)?, Some(claims.exp)))
    }

    pub(super) async fn claims(&self, token: &str, api: bool) -> Result<Claims, AuthError> {
        if token.len() > 32 * 1024 {
            return Err(AuthError::Unauthorized);
        }
        let header = decode_header(token).map_err(|_| AuthError::Unauthorized)?;
        if header.alg != Algorithm::RS256
            || (api && !matches!(header.typ.as_deref(), Some("at+jwt" | "application/at+jwt")))
        {
            return Err(AuthError::Unauthorized);
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|k| k.len() <= 256)
            .ok_or(AuthError::Unauthorized)?;
        let mut provider = self.provider.lock().await;
        let unknown = provider.keys.find(kid).is_none();
        provider.current(&self.http, &self.config, unknown).await?;
        let jwk = provider.keys.find(kid).ok_or(AuthError::Unauthorized)?;
        if !super::provider::usable_key(jwk) {
            return Err(AuthError::Unauthorized);
        }
        let key = DecodingKey::from_jwk(jwk).map_err(|_| AuthError::Unauthorized)?;
        drop(provider);
        let audience = if api {
            &self.config.audience
        } else {
            &self.config.client_id
        };
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp", "iat"]);
        validation.validate_nbf = true;
        validation.leeway = 60;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| AuthError::Unauthorized)?
            .claims;
        let now = jsonwebtoken::get_current_timestamp();
        if claims.iss != self.config.issuer
            || !claims.aud.matches(audience)
            || claims.sub.is_empty()
            || claims.sub.len() > 256
            || claims.iat > now.saturating_add(60)
            || claims.exp <= claims.iat
            || claims.nbf.is_some_and(|n| n > now.saturating_add(60))
        {
            return Err(AuthError::Unauthorized);
        }
        if !api
            && (claims
                .azp
                .as_ref()
                .is_some_and(|v| v != &self.config.client_id)
                || (claims.aud.multiple() && claims.azp.is_none()))
        {
            return Err(AuthError::Unauthorized);
        }
        if api
            && [claims.client_id.as_deref(), claims.jti.as_deref()]
                .iter()
                .any(|v| v.is_none_or(|v| v.is_empty() || v.len() > 256))
        {
            return Err(AuthError::Unauthorized);
        }
        Ok(claims)
    }

    pub(super) fn identity(&self, claims: &Claims, api: bool) -> Result<Authenticated, AuthError> {
        let principal = serde_json::to_string(&("oidc-v1", &claims.iss, &claims.sub))
            .map_err(|_| AuthError::Unauthorized)?;
        let mut scopes = self.config.scopes(&claims.sub);
        if api {
            let permitted: Vec<_> = claims
                .scope
                .as_deref()
                .unwrap_or("")
                .split_ascii_whitespace()
                .collect();
            scopes.retain(|s| permitted.contains(&s.as_str()));
        }
        let name = claims
            .name
            .as_deref()
            .filter(|n| !n.is_empty() && n.len() <= 256 && !n.chars().any(char::is_control))
            .unwrap_or(&claims.sub)
            .to_owned();
        Ok(Authenticated {
            actor: Actor {
                name: principal,
                scopes,
            },
            name,
            csrf: None,
        })
    }
}
