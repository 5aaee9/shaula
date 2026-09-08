//! Provider token contracts exercised through HTTPS exchange and the real router.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[path = "oidc_refresh_tokens/invalid.rs"]
mod invalid;
#[path = "support/oidc_renewal.rs"]
mod renewal;

use axum::{body::to_bytes, http::StatusCode};
use common::oidc::TokenControl;
use renewal::Harness;
use serde_json::{json, Value};

fn renewable() -> TokenControl {
    TokenControl {
        issue_refresh: true,
        expires_in: Some(1),
        ..TokenControl::default()
    }
}

async fn session(harness: &Harness) -> Value {
    let response = harness.send("GET", "/api/v1/session", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-csrf-token"], harness.csrf);
    assert!(response.headers().get("set-cookie").is_none());
    assert!(response.headers().get("location").is_none());
    serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await.unwrap()).unwrap()
}

/// A rejected successful exchange must remove the session, even if the Provider
/// already consumed the submitted rotating token. A later request cannot retry it.
async fn rejected_without_retry(harness: &Harness) {
    harness.expire().await;
    let response = harness.send("GET", "/api/v1/session", &[]).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    assert_eq!(harness.refreshes(), 1);
    {
        let mut control = harness.provider.control.lock().unwrap();
        control.tokens = renewable();
        control.keys_offline = false;
    }
    assert_eq!(
        harness.send("GET", "/api/v1/session", &[]).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        harness.refreshes(),
        1,
        "terminal sessions cannot retry old tokens"
    );
}

#[tokio::test]
async fn oidc_tests_refresh_without_id_token_preserves_identity_and_rotates_tokens() {
    let harness = Harness::new(TokenControl {
        omit_refresh_id_token: true,
        refresh_response: json!({"scope": null}).as_object().unwrap().clone(),
        ..renewable()
    })
    .await;
    let initial = session(&harness).await;
    for expected_exchanges in 1..=2 {
        harness.expire().await;
        assert_eq!(session(&harness).await, initial);
        assert_eq!(harness.refreshes(), expected_exchanges);
    }
    let requests = harness.provider.requests.lock().unwrap();
    let refresh: Vec<_> = requests
        .iter()
        .filter(|request| request["grant_type"] == "refresh_token")
        .collect();
    assert_eq!(refresh.len(), 2);
    assert_ne!(refresh[0]["refresh_token"], refresh[1]["refresh_token"]);
    assert!(refresh.iter().all(|request| request.get("scope").is_none()));
    let rendered = initial.to_string();
    assert!(!rendered.contains("refresh-"));
    assert!(!rendered.contains("refreshed-access"));
}

#[tokio::test]
async fn oidc_tests_refresh_omitted_replacement_reuses_the_original_token() {
    let harness = Harness::new(TokenControl {
        rotate_refresh: false,
        omit_refresh_nonce: true,
        ..renewable()
    })
    .await;
    for _ in 0..2 {
        harness.expire().await;
        session(&harness).await;
    }
    let requests = harness.provider.requests.lock().unwrap();
    let refresh: Vec<_> = requests
        .iter()
        .filter(|request| request["grant_type"] == "refresh_token")
        .collect();
    assert_eq!(refresh.len(), 2);
    assert_eq!(refresh[0]["refresh_token"], refresh[1]["refresh_token"]);
}

#[tokio::test]
async fn oidc_tests_refresh_accepts_original_audience_values_in_a_different_order() {
    let harness = Harness::new(TokenControl {
        login_claims: json!({"aud": ["web", "other"], "azp": "web"})
            .as_object()
            .unwrap()
            .clone(),
        refresh_claims: json!({"aud": ["other", "web"]})
            .as_object()
            .unwrap()
            .clone(),
        ..renewable()
    })
    .await;
    harness.expire().await;
    session(&harness).await;
    assert_eq!(harness.refreshes(), 1);
}

#[tokio::test]
async fn oidc_tests_refresh_id_token_supplies_lease_when_expires_in_is_omitted() {
    let harness = Harness::new(renewable()).await;
    harness.provider.control.lock().unwrap().tokens.expires_in = None;
    harness.expire().await;
    session(&harness).await;
    assert_eq!(harness.refreshes(), 1);
    harness.expire().await;
    session(&harness).await;
    assert_eq!(
        harness.refreshes(),
        1,
        "the new ID Token grants a future lease"
    );
}

#[tokio::test]
async fn oidc_tests_refresh_preserves_original_auth_time_and_configured_permissions() {
    let harness = Harness::new(TokenControl {
        login_claims: json!({"auth_time": 1_700_000_000})
            .as_object()
            .unwrap()
            .clone(),
        refresh_claims: json!({"name": "Updated operator", "scope": "super.admin"})
            .as_object()
            .unwrap()
            .clone(),
        refresh_response: json!({"scope": "openid"}).as_object().unwrap().clone(),
        omit_refresh_nonce: true,
        ..renewable()
    })
    .await;
    let initial = session(&harness).await;
    harness.expire().await;
    let refreshed = session(&harness).await;
    assert_eq!(refreshed["name"], "Updated operator");
    assert_eq!(refreshed["scopes"], initial["scopes"]);
    assert_eq!(harness.refreshes(), 1);
}

#[tokio::test]
async fn oidc_tests_provider_without_refresh_token_retains_finite_login() {
    let harness = Harness::new(TokenControl {
        login_claims: json!({"exp": jsonwebtoken::get_current_timestamp() + 4})
            .as_object()
            .unwrap()
            .clone(),
        ..TokenControl::default()
    })
    .await;
    session(&harness).await;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    assert_eq!(
        harness.send("GET", "/api/v1/session", &[]).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(harness.refreshes(), 0);
}
