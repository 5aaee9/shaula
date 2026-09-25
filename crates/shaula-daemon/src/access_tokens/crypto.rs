use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use shaula_core::access_tokens::{TokenError, TokenResult};
use zeroize::Zeroizing;

pub(super) const PREFIX: &str = "shaula_pat_v1_";

pub(super) fn parse(raw: &str) -> TokenResult<(&str, Zeroizing<Vec<u8>>)> {
    let raw = raw.strip_prefix(PREFIX).ok_or(TokenError::Unauthorized)?;
    if raw.len() != 76 || raw.as_bytes()[32] != b'_' {
        return Err(TokenError::Unauthorized);
    }
    let id = raw.get(..32).ok_or(TokenError::Unauthorized)?;
    if !id
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(TokenError::Unauthorized);
    }
    let encoded = raw.get(33..).ok_or(TokenError::Unauthorized)?;
    let secret = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| TokenError::Unauthorized)?,
    );
    if secret.len() != 32 || URL_SAFE_NO_PAD.encode(&secret) != encoded {
        return Err(TokenError::Unauthorized);
    }
    Ok((id, secret))
}

pub(super) fn mint() -> TokenResult<(String, Zeroizing<[u8; 32]>, String)> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut secret = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(secret.as_mut()).map_err(|_| TokenError::Unavailable)?;
    let raw = format!("{PREFIX}{id}_{}", URL_SAFE_NO_PAD.encode(*secret));
    Ok((id, secret, raw))
}

pub(super) fn digest(realm: &str, id: &str, secret: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    for part in [
        b"shaula-personal-access-token-v1".as_slice(),
        realm.as_bytes(),
        id.as_bytes(),
        secret,
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().to_vec()
}
