#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::oidc::{support, Oidc};
use serde_json::json;

#[tokio::test]
async fn oidc_tests_discovery_rejects_mismatch_insecure_endpoints_and_capabilities() {
    let fixture = support::start_provider();
    for (key, value) in [
        ("issuer", json!("https://wrong.example")),
        ("token_endpoint", json!("http://localhost/token")),
        ("response_types_supported", json!(["token"])),
        ("id_token_signing_alg_values_supported", json!(["HS256"])),
        ("token_endpoint_auth_methods_supported", json!(["none"])),
        ("code_challenge_methods_supported", json!(["plain"])),
    ] {
        fixture
            .control
            .lock()
            .unwrap()
            .metadata
            .insert(key.into(), value);
        let result = Oidc::discover_with_roots(
            fixture.config("https://shaula.example"),
            vec![reqwest::Certificate::from_pem(&fixture.certificate).unwrap()],
        )
        .await;
        assert!(result.is_err(), "{key}");
        fixture.control.lock().unwrap().metadata.clear();
    }
    for keys in [
        json!({"keys":[]}),
        json!({"keys":[{"kty":"oct","k":"c2VjcmV0","kid":"key","alg":"HS256"}]}),
    ] {
        fixture.control.lock().unwrap().keys = Some(keys);
        assert!(Oidc::discover_with_roots(
            fixture.config("https://shaula.example"),
            vec![reqwest::Certificate::from_pem(&fixture.certificate).unwrap()]
        )
        .await
        .is_err());
    }
    assert!(
        Oidc::discover(fixture.config("https://shaula.example"))
            .await
            .is_err(),
        "untrusted TLS certificate"
    );
}

#[tokio::test]
async fn oidc_tests_rotation_unknown_key_dedup_and_expired_cache_fail_closed() {
    let fixture = support::start_provider();
    let oidc = fixture.oidc().await;
    let claims = support::claims(&fixture.issuer, "api", "ops", support::SCOPES);
    fixture.control.lock().unwrap().keys = Some(
        json!({"keys":[{"kty":"RSA","n":support::MODULUS,"e":"AQAB","kid":"rotated","alg":"RS256","use":"sig"}]}),
    );
    let rotated = support::sign(claims.clone(), "at+jwt", "rotated");
    assert!(oidc.claims(&rotated, true).await.is_ok());
    let unknown = support::sign(claims, "at+jwt", "unknown");
    let futures = (0..16).map(|_| oidc.claims(&unknown, true));
    assert!(futures::future::join_all(futures)
        .await
        .iter()
        .all(Result::is_err));
    assert_eq!(
        fixture.control.lock().unwrap().key_reads,
        2,
        "one refresh for concurrent unknown keys"
    );
    fixture.control.lock().unwrap().offline = true;
    {
        let mut cache = oidc.provider.lock().await;
        cache.attempted -= BACKOFF;
    }
    assert!(oidc.claims(&unknown, true).await.is_err());
    assert!(!oidc.ready().await);
    assert!(
        oidc.claims(&rotated, true).await.is_ok(),
        "valid cache survives outage"
    );
    {
        let mut cache = oidc.provider.lock().await;
        cache.loaded -= TTL;
    }
    assert!(
        oidc.claims(&rotated, true).await.is_err(),
        "expired keys cannot authenticate"
    );
    fixture.control.lock().unwrap().offline = false;
    oidc.provider.lock().await.attempted -= BACKOFF;
    assert!(oidc.ready().await);
    assert!(oidc.claims(&rotated, true).await.is_ok());
}

#[tokio::test]
async fn oidc_tests_id_token_nonce_and_authorized_party_claims() {
    let fixture = support::start_provider();
    let oidc = fixture.oidc().await;
    let mut claims = support::claims(&fixture.issuer, "web", "ops", support::SCOPES);
    claims["aud"] = json!(["web", "other"]);
    assert!(oidc
        .claims(&support::sign(claims.clone(), "JWT", "test-key"), false)
        .await
        .is_err());
    claims["azp"] = json!("other");
    assert!(oidc
        .claims(&support::sign(claims.clone(), "JWT", "test-key"), false)
        .await
        .is_err());
    claims["azp"] = json!("web");
    assert!(oidc
        .browser_identity(&support::sign(claims.clone(), "JWT", "test-key"), "nonce")
        .await
        .is_err());
    claims["nonce"] = json!("nonce");
    let token = support::sign(claims.clone(), "JWT", "test-key");
    assert!(oidc.browser_identity(&token, "wrong-nonce").await.is_err());
    assert!(oidc.browser_identity(&token, "nonce").await.is_ok());
    let first = oidc
        .claims(&support::sign(claims.clone(), "JWT", "test-key"), false)
        .await
        .unwrap();
    claims["name"] = json!("Different display name");
    let second = oidc
        .claims(&support::sign(claims, "JWT", "test-key"), false)
        .await
        .unwrap();
    assert_eq!(
        oidc.identity(&first, false).unwrap().actor.name,
        oidc.identity(&second, false).unwrap().actor.name
    );
}

#[tokio::test]
async fn oidc_tests_unsigned_symmetric_and_tampered_tokens_are_rejected() {
    let fixture = support::start_provider();
    let oidc = fixture.oidc().await;
    let claims = support::claims(&fixture.issuer, "api", "ops", support::SCOPES);
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    header.typ = Some("at+jwt".into());
    header.kid = Some("test-key".into());
    let symmetric = jsonwebtoken::encode(
        &header,
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(b"test-secret"),
    )
    .unwrap();
    assert!(oidc.claims(&symmetric, true).await.is_err());
    let signed = support::sign(claims.clone(), "at+jwt", "test-key");
    let (input, _) = signed.rsplit_once('.').unwrap();
    assert!(oidc.claims(&format!("{input}.AAAA"), true).await.is_err());
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(br#"{"alg":"none","typ":"at+jwt","kid":"test-key"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    assert!(oidc
        .claims(&format!("{header}.{payload}."), true)
        .await
        .is_err());
}
