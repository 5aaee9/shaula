//! Real router and HTTPS Provider fixture shared by renewal contract tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crate::common::{self, oidc};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
    Router,
};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

pub struct Harness {
    pub provider: oidc::Provider,
    pub app: Router,
    pub cookie: String,
    pub csrf: String,
}

impl Harness {
    pub async fn new(tokens: oidc::TokenControl) -> Self {
        let provider = oidc::start_provider();
        provider.control.lock().unwrap().tokens = tokens;
        let (_, _, _, service) = common::build_app_with_service().await;
        let artifact_root = tempfile::tempdir().unwrap().keep();
        let app = shaula_http::router::build_router(shaula_http::router::AppState {
            auth_installation_link: None,
            fleets: service.clone(),
            profiles: service.clone(),
            health: service,
            oidc: provider.oidc().await,
            body_limit: 1024 * 1024,
            request_body_limit: 1024 * 1024,
            jobs: None,
            logs: None,
            artifact_publisher: Arc::new(common::TestPublisher {
                root: artifact_root,
            }),
        });
        let mut h = Self {
            provider,
            app,
            cookie: String::new(),
            csrf: String::new(),
        };
        (h.cookie, h.csrf) = h.login_replacing().await;
        h
    }

    pub async fn login_replacing(&self) -> (String, String) {
        self.login_with_cookie(&self.cookie).await
    }

    pub async fn login_fresh(&self) -> (String, String) {
        self.login_with_cookie("").await
    }

    async fn login_with_cookie(&self, old: &str) -> (String, String) {
        let start = self
            .send(
                "GET",
                "/auth/oidc/login?return_to=%2Fauth%3Fkey%3Ddraft",
                &[],
            )
            .await;
        assert_eq!(start.status(), StatusCode::FOUND);
        let authorization_url =
            url::Url::parse(start.headers()["location"].to_str().unwrap()).unwrap();
        let scope = authorization_url
            .query_pairs()
            .find(|(key, _)| key == "scope")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(
            scope
                .split_whitespace()
                .collect::<std::collections::BTreeSet<_>>(),
            ["openid", "profile"].into_iter().collect()
        );
        let binding = start.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(&self.provider.certificate).unwrap(),
            )
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let authorization = client
            .get(start.headers()["location"].to_str().unwrap())
            .send()
            .await
            .unwrap();
        let target =
            url::Url::parse(authorization.headers()["location"].to_str().unwrap()).unwrap();
        let path = format!("{}?{}", target.path(), target.query().unwrap());
        let cookies = if old.is_empty() {
            binding.to_owned()
        } else {
            format!("{binding}; {old}")
        };
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&path)
                    .header("cookie", cookies)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let raw = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .find(|v| v.to_str().unwrap().starts_with("__Host-shaula-session="))
            .unwrap()
            .to_str()
            .unwrap();
        for attribute in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/"] {
            assert!(raw.contains(attribute));
        }
        assert!(!raw.contains("Domain="));
        if self.provider.control.lock().unwrap().tokens.issue_refresh {
            assert!(!raw.contains("Max-Age="));
            assert!(!raw.contains("Expires="));
        }
        let cookie = raw.split(';').next().unwrap().to_owned();
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/session")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        (
            cookie,
            response.headers()["x-csrf-token"]
                .to_str()
                .unwrap()
                .to_owned(),
        )
    }

    pub fn request(&self, method: &str, path: &str, csrf: bool) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", &self.cookie);
        if csrf {
            builder = builder
                .header("origin", "https://shaula.example")
                .header("x-csrf-token", &self.csrf);
        }
        builder.body(Body::empty()).unwrap()
    }

    pub async fn send(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Response {
        let mut request = self.request(method, path, false);
        for (key, value) in headers {
            request.headers_mut().append(
                axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        response
    }

    pub async fn expire(&self) {
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }

    pub fn refreshes(&self) -> usize {
        self.provider.control.lock().unwrap().refresh_requests
    }

    pub async fn wait_refresh(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.refreshes() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
