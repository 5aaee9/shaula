#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use common::oidc;
use serde_json::json;
use tower::ServiceExt;

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(path);
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    response
}

async fn start(app: &Router, target: &str) -> (String, String) {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("return_to", target)
        .finish();
    let response = send(
        app,
        "GET",
        &format!("/auth/oidc/login?{query}"),
        &[("host", "attacker.example")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let binding = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let authorization = response.headers()["location"].to_str().unwrap();
    let params: std::collections::HashMap<_, _> = url::Url::parse(authorization)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(
        params["redirect_uri"],
        "https://shaula.example/auth/oidc/callback"
    );
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(!params.contains_key("client_secret"));
    let client = reqwest::Client::builder()
        .add_root_certificate(
            reqwest::Certificate::from_pem(&oidc::provider().certificate).unwrap(),
        )
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client.get(authorization).send().await.unwrap();
    let callback = url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    (
        binding,
        format!("{}?{}", callback.path(), callback.query().unwrap()),
    )
}

async fn login(app: &Router) -> (String, String) {
    let (binding, callback) = start(app, "/auth?key=production").await;
    let response = send(app, "GET", &callback, &[("cookie", &binding)]).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()["location"], "/auth?key=production");
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .find(|v| v.to_str().unwrap().starts_with("__Host-shaula-session="))
        .unwrap()
        .to_str()
        .unwrap();
    for attribute in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/", "Max-Age="] {
        assert!(cookie.contains(attribute));
    }
    assert!(!cookie.contains("Domain="));
    let session = cookie.split(';').next().unwrap().to_owned();
    let response = send(app, "GET", "/api/v1/session", &[("cookie", &session)]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let csrf = response.headers()["x-csrf-token"]
        .to_str()
        .unwrap()
        .to_owned();
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["name"], "Test operator");
    assert!(body["scopes"]
        .as_array()
        .unwrap()
        .contains(&json!("auth.write")));
    assert_eq!(
        send(app, "GET", &callback, &[("cookie", &binding)])
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    (session, csrf)
}

#[tokio::test]
async fn oidc_tests_default_deny_including_assets_methods_and_fallback() {
    let (app, _, _) = common::build_app_with_scan().await;
    for (method, path) in [
        ("GET", "/assets/missing.js"),
        ("GET", "/favicon.ico"),
        ("GET", "/robots.txt"),
        ("GET", "/livez"),
        ("GET", "/readyz"),
        ("GET", "/unknown"),
        ("GET", "/api/v1/unknown"),
        ("HEAD", "/fleets"),
        ("OPTIONS", "/api/v1/fleets"),
        ("POST", "/auth/oidc/login"),
        ("HEAD", "/auth/oidc/login"),
        ("POST", "/auth/oidc/callback"),
        ("POST", "/auth/oidc/logout"),
    ] {
        let response = send(&app, method, path, &[]).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
    }
    let bearer = oidc::bearer(oidc::SCOPES);
    assert_eq!(
        send(&app, "GET", "/fleets", &[("authorization", &bearer)])
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/api/v1/unknown",
            &[("authorization", &bearer)]
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            "OPTIONS",
            "/api/v1/fleets",
            &[("authorization", &bearer)]
        )
        .await
        .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn oidc_tests_browser_session_csrf_logout_and_no_credential_fallback() {
    let (app, _, _) = common::build_app_with_scan().await;
    let (cookie, csrf) = login(&app).await;
    assert_eq!(
        send(&app, "GET", "/fleets", &[("cookie", &cookie)])
            .await
            .status(),
        StatusCode::OK
    );
    for headers in [
        vec![("cookie", cookie.as_str())],
        vec![
            ("cookie", cookie.as_str()),
            ("origin", "https://evil.example"),
            ("x-csrf-token", csrf.as_str()),
        ],
        vec![
            ("cookie", cookie.as_str()),
            ("origin", "https://shaula.example"),
            ("x-csrf-token", "incorrect"),
        ],
    ] {
        assert_eq!(
            send(&app, "POST", "/auth/oidc/logout", &headers)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    for bearer in ["Bearer invalid".to_owned(), oidc::bearer(oidc::SCOPES)] {
        assert_eq!(
            send(
                &app,
                "GET",
                "/api/v1/session",
                &[("cookie", &cookie), ("authorization", &bearer)]
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let response = send(
        &app,
        "POST",
        "/auth/oidc/logout",
        &[
            ("cookie", &cookie),
            ("origin", "https://shaula.example"),
            ("x-csrf-token", &csrf),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        send(&app, "GET", "/api/v1/session", &[("cookie", &cookie)])
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn oidc_tests_callback_binding_and_return_target_validation() {
    let (app, _, _) = common::build_app_with_scan().await;
    let (binding, callback) = start(&app, "https://evil.example/").await;
    assert_eq!(
        send(&app, "GET", &callback, &[]).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let response = send(&app, "GET", &callback, &[("cookie", &binding)]).await;
    assert_eq!(response.headers()["location"], "/fleets");
    let (_, callback) = start(&app, "/fleets").await;
    assert_eq!(
        send(
            &app,
            "GET",
            &callback,
            &[("cookie", "__Host-shaula-login=wrong")]
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn oidc_tests_signed_token_claims_fail_closed() {
    let (app, _, _) = common::build_app_with_scan().await;
    let base = oidc::claims(&oidc::provider().issuer, "api", "ops", oidc::SCOPES);
    let now = jsonwebtoken::get_current_timestamp();
    for (key, value) in [
        ("iss", json!("https://evil.example")),
        ("aud", json!("web")),
        ("sub", json!("")),
        ("exp", json!(now - 61)),
        ("iat", json!(now + 61)),
        ("nbf", json!(now + 61)),
        ("client_id", json!(null)),
        ("jti", json!(null)),
    ] {
        let mut claims = base.clone();
        claims[key] = value;
        let bearer = format!("Bearer {}", oidc::sign(claims, "at+jwt", "test-key"));
        assert_eq!(
            send(
                &app,
                "GET",
                "/api/v1/session",
                &[("authorization", &bearer)]
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED,
            "{key}"
        );
    }
    for (typ, kid) in [("JWT", "test-key"), ("at+jwt", "unknown")] {
        let bearer = format!("Bearer {}", oidc::sign(base.clone(), typ, kid));
        assert_eq!(
            send(
                &app,
                "GET",
                "/api/v1/session",
                &[("authorization", &bearer)]
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn oidc_tests_server_grants_and_token_scopes_intersect() {
    let (app, _, _) = common::build_app_with_scan().await;
    for (subject, scope, expected) in [
        ("ops", "openid", StatusCode::FORBIDDEN),
        ("unmapped", oidc::SCOPES, StatusCode::FORBIDDEN),
        ("ops", "fleet.read", StatusCode::OK),
    ] {
        let bearer = format!(
            "Bearer {}",
            oidc::sign(
                oidc::claims(&oidc::provider().issuer, "api", subject, scope),
                "at+jwt",
                "test-key"
            )
        );
        let response = send(
            &app,
            "GET",
            "/api/v1/fleets",
            &[
                ("authorization", &bearer),
                ("x-shaula-scopes", oidc::SCOPES),
                ("x-shaula-actor", "ops"),
            ],
        )
        .await;
        assert_eq!(response.status(), expected);
    }
}
