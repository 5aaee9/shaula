//! User credentials are separate from downstream platform credentials.
use crate::{registry::Actor, secret::SecretString};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TokenPolicy {
    pub enabled: bool,
    pub default_ttl_secs: u64,
    pub max_ttl_secs: u64,
    pub max_active_per_principal: u64,
    pub max_active_total: u64,
}
impl Default for TokenPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            default_ttl_secs: 7_776_000,
            max_ttl_secs: 31_536_000,
            max_active_per_principal: 20,
            max_active_total: 10_000,
        }
    }
}
impl TokenPolicy {
    pub fn validate(&self) -> bool {
        (60..=31_536_000).contains(&self.max_ttl_secs)
            && (60..=self.max_ttl_secs).contains(&self.default_ttl_secs)
            && self.max_active_per_principal > 0
            && self.max_active_total > 0
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub id: String,
    pub owner_issuer: String,
    pub owner_subject: String,
    pub owner_principal: String,
    pub realm: String,
    pub format_version: u8,
    pub name: String,
    pub secret_digest: Vec<u8>,
    pub scopes: Vec<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub revoked_by_principal: Option<String>,
    pub last_used_at_ms: Option<i64>,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub effective_scopes: Vec<String>,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    pub state: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueToken {
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_in_seconds: Option<u64>,
}

pub struct IssuedToken {
    pub access_token: TokenMetadata,
    pub secret: Option<SecretString>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenQuery {
    pub state: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenPage {
    pub items: Vec<TokenMetadata>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("authentication required")]
    Unauthorized,
    #[error("authentication unavailable")]
    Unavailable,
    #[error("scope denied")]
    Forbidden,
    #[error("primary authentication required")]
    PrimaryRequired,
    #[error("personal token required")]
    PersonalRequired,
    #[error("token not found")]
    NotFound,
    #[error("invalid token request")]
    Invalid,
    #[error("scope cannot be delegated")]
    NonDelegable,
    #[error("access tokens disabled")]
    Disabled,
    #[error("idempotency conflict")]
    Conflict,
    #[error("token quota exceeded")]
    Quota,
    #[error("issuance rate exceeded")]
    RateLimited,
    #[error("precondition required")]
    PreconditionRequired,
    #[error("precondition failed")]
    PreconditionFailed,
}
pub type TokenResult<T> = Result<T, TokenError>;

#[async_trait]
pub trait TokenStore: Send + Sync {
    async fn get(&self, id: &str) -> TokenResult<Option<TokenRecord>>;
    /// Atomically check replay, rate, quotas and commit verifier, receipt and audit.
    async fn issue(
        &self,
        record: TokenRecord,
        key: &str,
        hash: &str,
        policy: &TokenPolicy,
        authentication: &crate::registry::AuthenticationContext,
    ) -> TokenResult<(TokenRecord, bool)>;
    async fn list(
        &self,
        owner: &str,
        before: Option<(i64, String)>,
        limit: usize,
    ) -> TokenResult<Vec<TokenRecord>>;
    async fn revoke(
        &self,
        actor: &crate::registry::Actor,
        id: &str,
        revision: Option<i64>,
        now: i64,
    ) -> TokenResult<()>;
    async fn touch(&self, id: &str, now: i64) -> TokenResult<()>;
}

#[async_trait]
pub trait TokenService: Send + Sync {
    fn enabled(&self) -> bool;
    async fn authenticate(&self, raw: &str) -> TokenResult<(Actor, String)>;
    async fn issue(
        &self,
        actor: &Actor,
        primary: bool,
        body: IssueToken,
        key: &str,
    ) -> TokenResult<IssuedToken>;
    async fn get(&self, owner: &str, id: &str) -> TokenResult<TokenMetadata>;
    async fn list(&self, owner: &str, query: TokenQuery) -> TokenResult<TokenPage>;
    async fn revoke(&self, actor: &Actor, id: &str, revision: Option<i64>) -> TokenResult<()>;
}
