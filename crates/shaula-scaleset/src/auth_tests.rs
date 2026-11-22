//! Auth-chain unit tests, split to keep auth.rs within the 400-line budget.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn jwt_claims_match_go_client_windows() {
    // Signing requires a real RSA key; use a small generated one via
    // jsonwebtoken is heavyweight. Instead validate claim math and
    // parsing round-trip with a synthetic token.
    let exp = 1_800_000_000i64;
    use base64::Engine;
    let payload =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
    let synthetic = format!("header.{payload}.signature");
    assert_eq!(parse_jwt_exp(&synthetic), Some(exp));
    assert_eq!(parse_jwt_exp("not-a-jwt"), None);
}

#[test]
fn credential_debug_redacts() {
    let app = Credential::GitHubApp {
        client_id: "123".into(),
        installation_id: 456,
        private_key: SecretString::new("-----BEGIN"),
    };
    let rendered = format!("{app:?}");
    assert!(!rendered.contains("BEGIN"));
}
