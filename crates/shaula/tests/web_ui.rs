#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[tokio::test]
async fn web_tests_shell_and_management_require_authentication() {
    let (app, _, _) = common::build_app_with_scan().await;
    for path in [
        "/",
        "/fleets/example",
        "/fleets/linux-x64.1_a",
        "/templates",
        "/templates/linux-build/update",
        "/auth",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert!(to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .is_empty());
    }
    for path in [
        "/api/v1/session",
        "/api/v1/fleets",
        "/api/v1/template-profiles",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .oneshot(common::authorized("GET", "/api/v1/session", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["name"].is_string());
    assert!(json["scopes"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("fleet.read")));
    assert_eq!(
        json.as_object().unwrap().len(),
        2,
        "session must expose only identity and scopes"
    );
}
