use super::{config::https_url, AuthError, OidcConfig};
use jsonwebtoken::jwk::JwkSet;
use serde::{de::DeserializeOwned, Deserialize};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(300);
const BACKOFF: Duration = Duration::from_secs(10);
const MAX_BODY: usize = 1024 * 1024;
#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;

#[derive(Clone, Deserialize)]
pub(super) struct Metadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub response_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
    pub scopes_supported: Option<Vec<String>>,
    pub code_challenge_methods_supported: Option<Vec<String>>,
}

pub(super) struct ProviderCache {
    pub metadata: Metadata,
    pub keys: JwkSet,
    loaded: Instant,
    attempted: Instant,
    pub(super) failed: bool,
}

impl ProviderCache {
    pub async fn load(http: &reqwest::Client, config: &OidcConfig) -> Result<Self, AuthError> {
        let discovery = format!(
            "{}/.well-known/openid-configuration",
            config.issuer.trim_end_matches('/')
        );
        let metadata: Metadata = json(http.get(discovery)).await?;
        if metadata.issuer != config.issuer
            || !metadata
                .response_types_supported
                .iter()
                .any(|s| s == "code")
            || metadata.subject_types_supported.is_empty()
            || !metadata
                .id_token_signing_alg_values_supported
                .iter()
                .any(|s| s == "RS256")
            || metadata
                .token_endpoint_auth_methods_supported
                .as_ref()
                .is_some_and(|s| !s.iter().any(|s| s == "client_secret_basic"))
            || metadata
                .scopes_supported
                .as_ref()
                .is_some_and(|s| !s.iter().any(|s| s == "openid"))
            || metadata
                .code_challenge_methods_supported
                .as_ref()
                .is_some_and(|s| !s.iter().any(|s| s == "S256"))
        {
            return Err(AuthError::Provider);
        }
        for endpoint in [
            &metadata.authorization_endpoint,
            &metadata.token_endpoint,
            &metadata.jwks_uri,
        ] {
            https_url(endpoint).map_err(|_| AuthError::Provider)?;
        }
        let keys: JwkSet = json(http.get(&metadata.jwks_uri)).await?;
        let mut kids = std::collections::HashSet::new();
        if keys.keys.is_empty()
            || keys.keys.len() > 64
            || keys
                .keys
                .iter()
                .filter_map(|k| k.common.key_id.as_ref())
                .any(|kid| !kids.insert(kid))
            || !keys.keys.iter().any(usable_key)
        {
            return Err(AuthError::Provider);
        }
        Ok(Self {
            metadata,
            keys,
            loaded: Instant::now(),
            attempted: Instant::now() - BACKOFF,
            failed: false,
        })
    }

    pub async fn current(
        &mut self,
        http: &reqwest::Client,
        config: &OidcConfig,
        unknown_key: bool,
    ) -> Result<(), AuthError> {
        let expired = self.loaded.elapsed() >= TTL;
        if !expired && !unknown_key {
            return Ok(());
        }
        if self.attempted.elapsed() < BACKOFF {
            return if expired || self.failed {
                Err(AuthError::Provider)
            } else {
                Ok(())
            };
        }
        self.attempted = Instant::now();
        match Self::load(http, config).await {
            Ok(mut new) => {
                new.attempted = Instant::now();
                *self = new;
                Ok(())
            }
            Err(e) => {
                self.failed = true;
                Err(e)
            }
        }
    }
}

pub(super) fn usable_key(key: &jsonwebtoken::jwk::Jwk) -> bool {
    use base64::Engine;
    use jsonwebtoken::jwk::{AlgorithmParameters, KeyAlgorithm, KeyOperations, PublicKeyUse};
    let AlgorithmParameters::RSA(parameters) = &key.algorithm else {
        return false;
    };
    let modulus = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&parameters.n);
    let strong_rsa = modulus.is_ok_and(|n| {
        let bits = n
            .len()
            .saturating_mul(8)
            .saturating_sub(n.first().map_or(8, |byte| byte.leading_zeros() as usize));
        (2048..=8192).contains(&bits)
    });
    strong_rsa
        && key
            .common
            .key_id
            .as_ref()
            .is_some_and(|kid| !kid.is_empty() && kid.len() <= 256)
        && key
            .common
            .public_key_use
            .as_ref()
            .is_none_or(|v| *v == PublicKeyUse::Signature)
        && key
            .common
            .key_algorithm
            .as_ref()
            .is_none_or(|v| *v == KeyAlgorithm::RS256)
        && key
            .common
            .key_operations
            .as_ref()
            .is_none_or(|v| v.contains(&KeyOperations::Verify))
        && jsonwebtoken::DecodingKey::from_jwk(key).is_ok()
}

pub(super) async fn json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, AuthError> {
    let response = bounded(request).await?;
    if !response.status().is_success() {
        return Err(AuthError::Provider);
    }
    serde_json::from_slice(response.body()).map_err(|_| AuthError::Provider)
}

pub(super) async fn bounded(
    request: reqwest::RequestBuilder,
) -> Result<http::Response<Vec<u8>>, AuthError> {
    let response = request.send().await.map_err(|_| AuthError::Provider)?;
    bounded_body(response, AuthError::Provider).await
}

pub(super) async fn bounded_body(
    mut response: reqwest::Response,
    oversized: AuthError,
) -> Result<http::Response<Vec<u8>>, AuthError> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(oversized);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| AuthError::Provider)? {
        if body.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(oversized);
        }
        body.extend_from_slice(&chunk);
    }
    let mut result = http::Response::builder().status(response.status());
    if let Some(headers) = result.headers_mut() {
        *headers = response.headers().clone();
    }
    result.body(body).map_err(|_| AuthError::Provider)
}
