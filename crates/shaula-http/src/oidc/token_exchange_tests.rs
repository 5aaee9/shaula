#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn oidc_tests_validation_latency_cannot_restart_the_access_token_lease() {
    let tokens: Tokens = serde_json::from_value(serde_json::json!({
        "access_token": "test-access", "token_type": "Bearer", "expires_in": 1,
    }))
    .unwrap();
    let received = Instant::now() - Duration::from_secs(2);
    assert!(expiry(
        &tokens,
        Some(jsonwebtoken::get_current_timestamp() + 3600),
        received
    )
    .is_err());
    assert!(expiry(&tokens, None, received).is_err());
}

#[test]
fn oidc_tests_access_deadline_is_anchored_to_response_receipt() {
    let tokens: Tokens = serde_json::from_value(serde_json::json!({
        "access_token": "test-access", "token_type": "Bearer", "expires_in": 300,
    }))
    .unwrap();
    let received = Instant::now() - Duration::from_secs(10);
    assert_eq!(
        expiry(&tokens, None, received).unwrap(),
        received + Duration::from_secs(300)
    );
}
