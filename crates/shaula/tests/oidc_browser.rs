//! Explicit browser harness: real daemon binary behind a local HTTPS proxy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "support/oidc_startup.rs"]
mod support;
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    response::{IntoResponse, Response},
    Router,
};
use support::*;

#[tokio::test]
#[ignore = "long-running HTTPS harness owned by web/oidc.playwright.config.ts"]
async fn oidc_browser_server() {
    let fixture = Startup::new();
    let mut child = Running(
        fixture
            .command()
            .env("SHAULA_OIDC_PUBLIC_URL", "https://localhost:5181")
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let target = format!("http://127.0.0.1:{}", fixture.port);
    for _ in 0..100 {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "daemon startup failed"
        );
        if client.get(format!("{target}/livez")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem(
        cert.cert.pem().into_bytes(),
        cert.key_pair.serialize_pem().into_bytes(),
    )
    .await
    .unwrap();
    let app = Router::new().fallback(proxy).with_state((client, target));
    axum_server::bind_rustls("127.0.0.1:5181".parse().unwrap(), tls)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
async fn proxy(
    State((client, target)): State<(reqwest::Client, String)>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 1024 * 1024).await.unwrap();
    let response = client
        .request(
            parts.method,
            format!(
                "{target}{}",
                parts.uri.path_and_query().map_or("/", |p| p.as_str())
            ),
        )
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let mut result = (status, Body::from(response.bytes().await.unwrap())).into_response();
    *result.headers_mut() = headers;
    result
}
