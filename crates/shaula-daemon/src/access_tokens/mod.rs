//! Personal credential use cases. No network calls or positive auth cache.
mod crypto;
mod reads;
use async_trait::async_trait;
use shaula_core::{
    access_tokens::*,
    ports::Clock,
    registry::{Actor, Scope},
    secret::SecretString,
};
use std::{collections::BTreeMap, sync::Arc};
use subtle::ConstantTimeEq;

pub struct PersonalTokens {
    store: Arc<dyn TokenStore>,
    clock: Arc<dyn Clock>,
    policy: TokenPolicy,
    realm: String,
    grants: BTreeMap<String, Vec<Scope>>,
    verification: tokio::sync::Semaphore,
}

impl PersonalTokens {
    pub fn new(
        store: Arc<dyn TokenStore>,
        clock: Arc<dyn Clock>,
        policy: TokenPolicy,
        realm: String,
        grants: BTreeMap<String, Vec<Scope>>,
    ) -> TokenResult<Self> {
        if !policy.validate() {
            return Err(TokenError::Invalid);
        }
        Ok(Self {
            store,
            clock,
            policy,
            realm,
            grants,
            verification: tokio::sync::Semaphore::new(64),
        })
    }

    fn active(&self, record: &TokenRecord, now: i64) -> bool {
        self.policy.enabled
            && record.format_version == 1
            && record.realm == self.realm
            && record.revoked_at_ms.is_none()
            && record.expires_at_ms > now
            && self.grants.contains_key(&record.owner_principal)
            && serde_json::to_string(&("oidc-v1", &record.owner_issuer, &record.owner_subject))
                .ok()
                .as_deref()
                == Some(record.owner_principal.as_str())
    }

    fn effective(&self, record: &TokenRecord) -> Vec<Scope> {
        self.grants
            .get(&record.owner_principal)
            .into_iter()
            .flatten()
            .copied()
            .filter(|scope| scope.delegable() && record.scopes.iter().any(|s| s == scope.as_str()))
            .collect()
    }
}

#[async_trait]
impl TokenService for PersonalTokens {
    fn enabled(&self) -> bool {
        self.policy.enabled
    }
    async fn authenticate(&self, raw: &str) -> TokenResult<(Actor, String)> {
        if !self.policy.enabled {
            return Err(TokenError::Unauthorized);
        }
        let (id, secret) = crypto::parse(raw)?;
        let _permit = self
            .verification
            .try_acquire()
            .map_err(|_| TokenError::Unavailable)?;
        let expected = crypto::digest(&self.realm, id, &secret);
        let record = self.store.get(id).await?;
        let stored = record
            .as_ref()
            .map(|r| r.secret_digest.as_slice())
            .unwrap_or(&[0; 32]);
        let equal = bool::from(expected.as_slice().ct_eq(stored));
        let record = record
            .filter(|r| equal && self.active(r, self.clock.now_unix_ms()))
            .ok_or(TokenError::Unauthorized)?;
        let actor = Actor {
            authentication: shaula_core::registry::AuthenticationContext::PersonalAccessToken(
                id.to_owned(),
            ),
            name: record.owner_principal.clone(),
            scopes: self.effective(&record),
        };
        let now = self.clock.now_unix_ms();
        if record
            .last_used_at_ms
            .is_none_or(|last| last <= now.saturating_sub(60_000))
        {
            let _ = self.store.touch(id, now).await;
        }
        Ok((actor, id.to_owned()))
    }

    async fn issue(
        &self,
        actor: &Actor,
        primary: bool,
        mut body: IssueToken,
        key: &str,
    ) -> TokenResult<IssuedToken> {
        if !primary {
            return Err(TokenError::PrimaryRequired);
        }
        if !actor.has(Scope::AccessTokenWrite) {
            return Err(TokenError::Forbidden);
        }
        if !self.policy.enabled {
            return Err(TokenError::Disabled);
        }
        if key.is_empty()
            || key.len() > 128
            || key.chars().any(char::is_control)
            || body.name.trim().is_empty()
            || body.name.len() > 128
            || body.name.chars().any(char::is_control)
        {
            return Err(TokenError::Invalid);
        }
        let (version, issuer, subject): (String, String, String) =
            serde_json::from_str(&actor.name).map_err(|_| TokenError::Invalid)?;
        if version != "oidc-v1" || !self.grants.contains_key(&actor.name) {
            return Err(TokenError::Forbidden);
        }
        body.scopes.sort();
        body.scopes.dedup();
        for s in &body.scopes {
            let scope = Scope::parse(s).ok_or(TokenError::Invalid)?;
            if !scope.delegable() {
                return Err(TokenError::NonDelegable);
            }
            if !actor.has(scope) {
                return Err(TokenError::Forbidden);
            }
        }
        // Hash the unresolved request, so a changed default TTL cannot break recovery.
        let canonical = serde_json::to_vec(&body).map_err(|_| TokenError::Invalid)?;
        let hash = shaula_core::auth::request_hash_parts(&[
            b"personal-token-issue-v1",
            actor.name.as_bytes(),
            &canonical,
        ]);
        let ttl = body
            .expires_in_seconds
            .unwrap_or(self.policy.default_ttl_secs);
        if !(60..=31_536_000).contains(&ttl) {
            return Err(TokenError::Invalid);
        }
        let now = self.clock.now_unix_ms();
        let expires = now
            .checked_add((ttl * 1000) as i64)
            .ok_or(TokenError::Invalid)?;
        let (id, secret, raw) = crypto::mint()?;
        let raw = SecretString::new(raw);
        let record = TokenRecord {
            secret_digest: crypto::digest(&self.realm, &id, secret.as_slice()),
            id,
            owner_issuer: issuer,
            owner_subject: subject,
            owner_principal: actor.name.clone(),
            realm: self.realm.clone(),
            format_version: 1,
            name: body.name,
            scopes: body.scopes,
            created_at_ms: now,
            expires_at_ms: expires,
            revoked_at_ms: None,
            revoked_by_principal: None,
            last_used_at_ms: None,
            revision: 1,
        };
        let (record, created) = self
            .store
            .issue(record, key, &hash, &self.policy, &actor.authentication)
            .await?;
        Ok(IssuedToken {
            access_token: self.metadata(&record),
            secret: created.then_some(raw),
        })
    }

    async fn get(&self, owner: &str, id: &str) -> TokenResult<TokenMetadata> {
        let record = self
            .store
            .get(id)
            .await?
            .filter(|r| r.owner_principal == owner)
            .ok_or(TokenError::NotFound)?;
        Ok(self.metadata(&record))
    }
    async fn list(&self, owner: &str, query: TokenQuery) -> TokenResult<TokenPage> {
        self.page(owner, query).await
    }
    async fn revoke(&self, actor: &Actor, id: &str, revision: Option<i64>) -> TokenResult<()> {
        self.store
            .revoke(actor, id, revision, self.clock.now_unix_ms())
            .await
    }
}
