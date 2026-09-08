use super::{rejected_without_retry, renewable, Harness};
use serde_json::{json, Value};

#[tokio::test]
async fn oidc_tests_refresh_rejects_changed_subject_issuer_and_audience() {
    for patch in [
        json!({"sub": "other-operator"}),
        json!({"iss": "https://other-provider.example"}),
        json!({"aud": "other-client"}),
        json!({"aud": ["web", "new-audience"], "azp": "web"}),
        json!({"azp": "other-client"}),
    ] {
        let harness = Harness::new(renewable()).await;
        harness
            .provider
            .control
            .lock()
            .unwrap()
            .tokens
            .refresh_claims = patch.as_object().unwrap().clone();
        rejected_without_retry(&harness).await;
    }
}

#[tokio::test]
async fn oidc_tests_refresh_rejects_changed_nonce_and_authentication_time() {
    for patch in [
        json!({"nonce": "a-different-login"}),
        json!({"auth_time": 1_700_000_001}),
    ] {
        let mut tokens = renewable();
        tokens
            .login_claims
            .insert("auth_time".into(), json!(1_700_000_000));
        tokens.refresh_claims = patch.as_object().unwrap().clone();
        rejected_without_retry(&Harness::new(tokens).await).await;
    }
    let mut tokens = renewable();
    tokens
        .refresh_claims
        .insert("auth_time".into(), json!(1_700_000_000));
    rejected_without_retry(&Harness::new(tokens).await).await;
}

#[tokio::test]
async fn oidc_tests_refresh_rejects_scope_expansion_and_missing_openid() {
    for scope in ["openid profile additional", "profile", ""] {
        let harness = Harness::new(renewable()).await;
        harness
            .provider
            .control
            .lock()
            .unwrap()
            .tokens
            .refresh_response
            .insert("scope".into(), json!(scope));
        rejected_without_retry(&harness).await;
    }
}

#[tokio::test]
async fn oidc_tests_refresh_rejects_absent_or_zero_lease() {
    for (seconds, omit_id_token) in [(None, true), (Some(0), true), (Some(0), false)] {
        let harness = Harness::new(renewable()).await;
        {
            let mut control = harness.provider.control.lock().unwrap();
            control.tokens.expires_in = seconds;
            control.tokens.omit_refresh_id_token = omit_id_token;
        }
        rejected_without_retry(&harness).await;
    }
}

#[tokio::test]
async fn oidc_tests_refresh_rejects_expired_or_future_issued_id_tokens() {
    let now = jsonwebtoken::get_current_timestamp();
    for patch in [
        json!({"iat": now - 3600, "exp": now - 1}),
        json!({"iat": now + 3600, "exp": now + 7200}),
        json!({"nbf": now + 3600}),
    ] {
        let harness = Harness::new(renewable()).await;
        harness
            .provider
            .control
            .lock()
            .unwrap()
            .tokens
            .refresh_claims = patch.as_object().unwrap().clone();
        rejected_without_retry(&harness).await;
    }
}

#[tokio::test]
async fn oidc_tests_refresh_malformed_success_terminates_the_rotated_session() {
    for (field, value) in [
        ("access_token", json!("")),
        ("access_token", Value::Null),
        ("access_token", json!("x".repeat(1024 * 1024 + 1))),
        ("refresh_token", json!("")),
        ("token_type", Value::Null),
        ("id_token", json!("not-a-jwt")),
        ("id_token", json!(42)),
        ("expires_in", json!("not-a-duration")),
    ] {
        let harness = Harness::new(renewable()).await;
        harness
            .provider
            .control
            .lock()
            .unwrap()
            .tokens
            .refresh_response
            .insert(field.into(), value);
        rejected_without_retry(&harness).await;
    }
}

#[tokio::test]
async fn oidc_tests_refresh_rotation_then_jwks_outage_terminates_without_old_token_retry() {
    let harness = Harness::new(renewable()).await;
    {
        let mut control = harness.provider.control.lock().unwrap();
        control.tokens.refresh_kid = Some("new-signing-key".into());
        control.keys_offline = true;
    }
    rejected_without_retry(&harness).await;
    assert!(harness.provider.control.lock().unwrap().key_reads >= 2);
}
