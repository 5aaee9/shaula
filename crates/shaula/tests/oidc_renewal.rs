//! Observable guard admission, singleflight, cancellation and revocation contracts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
#[path = "support/oidc_renewal.rs"]
mod renewal;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::oidc::TokenControl;
use renewal::Harness;
use std::time::Duration;
use tower::ServiceExt;

async fn short_session() -> Harness {
    Harness::new(TokenControl {
        issue_refresh: true,
        expires_in: Some(1),
        ..Default::default()
    })
    .await
}

#[tokio::test]
async fn concurrent_expired_requests_share_one_refresh_and_preserve_csrf() {
    let h = short_session().await;
    h.provider.control.lock().unwrap().tokens.expires_in = Some(3600);
    h.provider.control.lock().unwrap().tokens.delay_ms = 150;
    h.expire().await;
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let app = h.app.clone();
        let request = h.request("GET", "/api/v1/session", false);
        requests.spawn(async move { app.oneshot(request).await.unwrap() });
    }
    while let Some(response) = requests.join_next().await {
        let response = response.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-csrf-token"], h.csrf);
        assert!(!response.headers().contains_key("set-cookie"));
    }
    assert_eq!(h.refreshes(), 1);
    assert_eq!(
        h.send("GET", "/auth?key=draft", &[]).await.status(),
        StatusCode::OK
    );
    assert_eq!(h.refreshes(), 1);
}

#[tokio::test]
async fn unsafe_requests_validate_before_refresh_and_execute_only_after_success() {
    let h = short_session().await;
    h.provider.control.lock().unwrap().tokens.expires_in = Some(3600);
    h.expire().await;
    for headers in [
        vec![],
        vec![
            ("origin", "https://evil.example"),
            ("x-csrf-token", h.csrf.as_str()),
        ],
        vec![
            ("origin", "https://shaula.example"),
            ("x-csrf-token", "wrong"),
        ],
    ] {
        assert_eq!(
            h.send("PUT", "/api/v1/github-auth-profiles/renewed", &headers)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        h.send(
            "GET",
            "/api/v1/session",
            &[("authorization", "Bearer invalid")]
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(h.refreshes(), 0);
    let (mut parts, _) = h
        .request("PUT", "/api/v1/github-auth-profiles/renewed", true)
        .into_parts();
    parts.headers.insert("if-none-match", "*".parse().unwrap());
    parts
        .headers
        .insert("content-type", "application/json".parse().unwrap());
    parts
        .headers
        .insert("idempotency-key", "renew-once".parse().unwrap());
    let response = h
        .app
        .clone()
        .oneshot(Request::from_parts(
            parts,
            Body::from(common::AUTH_PUT_BODY),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(h.refreshes(), 1);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let accepted: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(accepted["revision"], 1);
    assert_eq!(
        h.send("GET", "/api/v1/github-auth-profiles/renewed", &[])
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn cancelled_waiter_does_not_lose_rotated_refresh_token() {
    let h = short_session().await;
    h.provider.control.lock().unwrap().tokens.delay_ms = 300;
    h.expire().await;
    let task = tokio::spawn(
        h.app
            .clone()
            .oneshot(h.request("GET", "/api/v1/session", false)),
    );
    h.wait_refresh().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        h.send("GET", "/api/v1/session", &[]).await.status(),
        StatusCode::OK
    );
    assert_eq!(h.refreshes(), 1);
    h.provider.control.lock().unwrap().tokens.expires_in = Some(3600);
    h.expire().await;
    assert_eq!(
        h.send("GET", "/api/v1/session", &[]).await.status(),
        StatusCode::OK
    );
    assert_eq!(h.refreshes(), 2);
    let requests = h.provider.requests.lock().unwrap();
    let tokens: Vec<_> = requests
        .iter()
        .filter_map(|r| r.get("refresh_token"))
        .collect();
    assert_eq!(tokens.len(), 2);
    assert_ne!(tokens[0], tokens[1]);
}

#[tokio::test]
async fn logout_during_refresh_wins_without_waiting_for_provider() {
    let h = short_session().await;
    h.provider.control.lock().unwrap().tokens.delay_ms = 1000;
    h.expire().await;
    let task = tokio::spawn(
        h.app
            .clone()
            .oneshot(h.request("GET", "/api/v1/session", false)),
    );
    h.wait_refresh().await;
    let response = tokio::time::timeout(
        Duration::from_millis(500),
        h.app
            .clone()
            .oneshot(h.request("POST", "/auth/oidc/logout", true)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        task.await.unwrap().unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.send("GET", "/api/v1/session", &[]).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(h.refreshes(), 1);
}

#[tokio::test]
async fn new_login_fences_old_refresh_and_other_sessions_remain_available() {
    let h = short_session().await;
    h.provider.control.lock().unwrap().tokens.expires_in = Some(3600);
    let (other, _) = h.login_fresh().await;
    h.provider.control.lock().unwrap().tokens.delay_ms = 1000;
    h.expire().await;
    let task = tokio::spawn(
        h.app
            .clone()
            .oneshot(h.request("GET", "/api/v1/session", false)),
    );
    h.wait_refresh().await;
    let request = Request::builder()
        .uri("/api/v1/session")
        .header("cookie", &other)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), h.app.clone().oneshot(request))
            .await
            .unwrap()
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let (replacement, _) = h.login_replacing().await;
    assert_ne!(replacement, h.cookie);
    assert_eq!(
        task.await.unwrap().unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let request = Request::builder()
        .uri("/api/v1/session")
        .header("cookie", replacement)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        h.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(h.refreshes(), 1);
}

#[tokio::test]
async fn temporary_failure_is_503_with_backoff_and_does_not_admit_a_mutation() {
    let h = short_session().await;
    h.provider.control.lock().unwrap().tokens.refresh_status = Some(503);
    h.expire().await;
    let (mut parts, _) = h
        .request("PUT", "/api/v1/github-auth-profiles/blocked", true)
        .into_parts();
    parts.headers.insert("if-none-match", "*".parse().unwrap());
    parts
        .headers
        .insert("content-type", "application/json".parse().unwrap());
    let response = h
        .app
        .clone()
        .oneshot(Request::from_parts(
            parts,
            Body::from(common::AUTH_PUT_BODY),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    for _ in 0..8 {
        assert_eq!(
            h.send("GET", "/api/v1/session", &[]).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    assert_eq!(h.refreshes(), 1);
    {
        let mut control = h.provider.control.lock().unwrap();
        control.tokens.refresh_status = None;
        control.tokens.expires_in = Some(3600);
    }
    tokio::time::sleep(Duration::from_millis(10_100)).await;
    assert_eq!(
        h.send("GET", "/api/v1/session", &[]).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        h.send("GET", "/api/v1/github-auth-profiles/blocked", &[])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(h.refreshes(), 2);
}
