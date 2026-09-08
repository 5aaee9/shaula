//! Explicit browser harness: real daemon binary behind a local HTTPS proxy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "support/oidc_startup.rs"]
mod support;
#[path = "support/oidc_browser_template.rs"]
mod template_fixture;
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use support::*;

#[derive(Clone)]
struct BrowserFixture {
    client: reqwest::Client,
    target: String,
    provider: Arc<Mutex<provider::Control>>,
    requests: Arc<Mutex<Vec<(String, String)>>>,
}

#[tokio::test]
#[ignore = "long-running HTTPS harness owned by web/oidc.playwright.config.ts"]
async fn oidc_browser_server() {
    let fixture = Startup::new();
    template_fixture::seed(fixture.directory.path())
        .await
        .unwrap();
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
    // These controls exist in this explicit test binary, never the daemon Router.
    let app = Router::new()
        .route(
            "/__test/provider",
            get(provider_status).post(configure_provider),
        )
        .fallback(proxy)
        .with_state(BrowserFixture {
            client,
            target,
            provider: fixture.provider.control.clone(),
            requests: Arc::default(),
        });
    axum_server::bind_rustls("127.0.0.1:5181".parse().unwrap(), tls)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
async fn proxy(State(fixture): State<BrowserFixture>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    fixture
        .requests
        .lock()
        .unwrap()
        .push((parts.method.to_string(), parts.uri.path().to_owned()));
    let body = to_bytes(body, 1024 * 1024).await.unwrap();
    let response = fixture
        .client
        .request(
            parts.method,
            format!(
                "{}{}",
                fixture.target,
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

async fn configure_provider(
    State(fixture): State<BrowserFixture>,
    Json(tokens): Json<provider::TokenControl>,
) -> axum::http::StatusCode {
    fixture.provider.lock().unwrap().tokens = tokens;
    axum::http::StatusCode::NO_CONTENT
}

async fn provider_status(State(fixture): State<BrowserFixture>) -> Json<Value> {
    let refresh_requests = fixture.provider.lock().unwrap().refresh_requests;
    let requests = fixture.requests.lock().unwrap();
    Json(json!({"refreshRequests": refresh_requests, "requests": *requests}))
}
