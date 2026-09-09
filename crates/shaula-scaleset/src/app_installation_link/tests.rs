use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::Router;
use serde_json::json;
use shaula_core::ports::Clock;

use super::*;

#[test]
fn installation_url_uses_only_the_verified_app_slug() {
    for slug in ["shaula", "a", "0", "shaula--runner-", &"a".repeat(100)] {
        assert_eq!(
            installation_url_for_app(
                "4863460",
                &json!({"id": 4863460, "slug": slug, "html_url": "https://evil.test"}),
            ),
            Ok(format!("https://github.com/apps/{slug}/installations/new"))
        );
    }
}

#[test]
fn installation_url_rejects_slug_path_and_origin_injection() {
    for slug in [
        "",
        "-shaula",
        "Shaula",
        " shaula",
        "shaula ",
        "shaula\n",
        "shaula\r",
        "shaula\t",
        "shaula/evil",
        "shaula\\evil",
        "shaula%2Fevil",
        "shaula%5Cevil",
        "shaula%0a",
        "shaula?next=evil",
        "shaula#evil",
        "https://evil.test",
        "//evil.test",
        "shaula@evil.test",
        "..",
        "shaula.runner",
        "shaula_runner",
        "sháulá",
        &"a".repeat(101),
    ] {
        let error = installation_url_for_app("4863460", &json!({"id": 4863460, "slug": slug}));
        assert_eq!(
            error,
            Err(ScalesetError::MalformedResponse {
                summary: "app installation metadata missing valid slug".into(),
            }),
            "must reject {slug:?} without exposing remote metadata"
        );
    }
    for slug in [json!(null), json!(1), json!([]), json!({})] {
        assert!(matches!(
            installation_url_for_app("4863460", &json!({"id": 4863460, "slug": slug})),
            Err(ScalesetError::MalformedResponse { .. })
        ));
    }
}

#[test]
fn installation_url_requires_complete_legal_app_identity() {
    for id in [
        json!(null),
        json!(0),
        json!(-1),
        json!(1.5),
        json!("4863460"),
        json!(9_007_199_254_740_992_i64),
        json!(u64::MAX),
    ] {
        assert!(matches!(
            installation_url_for_app("4863460", &json!({"id": id, "slug": "shaula"})),
            Err(ScalesetError::MalformedResponse { .. })
        ));
    }
    for body in [json!({}), json!([]), json!(null)] {
        assert!(matches!(
            installation_url_for_app("4863460", &body),
            Err(ScalesetError::MalformedResponse { .. })
        ));
    }
    for declared in ["1", "04863460", "4863460 ", "invalid"] {
        assert!(matches!(
            installation_url_for_app(declared, &json!({"id": 4863460, "slug": "shaula"})),
            Err(ScalesetError::Configuration { .. })
        ));
    }
}

struct Now;

impl Clock for Now {
    fn now_unix_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

struct Server {
    base: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start(
        status: StatusCode,
        headers: HeaderMap,
        body: String,
    ) -> Result<Self, std::io::Error> {
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let app = Router::new().fallback(move |method: Method, uri: Uri, request: HeaderMap| {
            count.fetch_add(1, Ordering::SeqCst);
            assert_eq!(method, Method::GET);
            assert_eq!(uri.path(), "/app");
            assert!(uri.query().is_none());
            assert!(request.get("authorization").is_some_and(|value| value
                .to_str()
                .is_ok_and(|value| value.starts_with("Bearer "))));
            assert!(request.contains_key("user-agent"));
            let reply = (status, headers.clone(), body.clone());
            async move { reply }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, app).await;
            assert!(result.is_ok());
        });
        Ok(Self {
            base,
            requests,
            task,
        })
    }

    async fn installation_url(&self) -> Result<String, ScalesetError> {
        let resolver = AppInstallationResolver::new(
            self.base.clone(),
            crate::client::production_http_client()?,
            Arc::new(Now),
        );
        resolver
            .installation_url(
                "4863460",
                &SecretString::new(include_str!("../../tests/fixtures/app.private.pem")),
            )
            .await
    }
}

#[tokio::test]
async fn installation_url_makes_one_authenticated_read_and_ignores_remote_urls(
) -> Result<(), Box<dyn std::error::Error>> {
    let server = Server::start(
        StatusCode::OK,
        HeaderMap::new(),
        json!({"id": 4863460, "slug": "shaula", "html_url": "https://evil.test"}).to_string(),
    )
    .await?;
    assert_eq!(
        server.installation_url().await?,
        "https://github.com/apps/shaula/installations/new"
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn installation_url_classifies_errors_without_disclosing_response_bodies(
) -> Result<(), Box<dyn std::error::Error>> {
    for (status, expected) in [
        (
            StatusCode::OK,
            ScalesetError::MalformedResponse {
                summary: "app installation metadata invalid".into(),
            },
        ),
        (
            StatusCode::UNAUTHORIZED,
            ScalesetError::Status {
                status: 401,
                summary: "app request failed".into(),
            },
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            ScalesetError::Status {
                status: 500,
                summary: "app request failed".into(),
            },
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            ScalesetError::RateLimited {
                retry_after_secs: Some(15),
                summary: "app request failed".into(),
            },
        ),
        (
            StatusCode::FOUND,
            ScalesetError::Status {
                status: 302,
                summary: "app request failed".into(),
            },
        ),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("15"));
        headers.insert("location", HeaderValue::from_static("/unexpected-redirect"));
        let server = Server::start(status, headers, "sensitive response body".into()).await?;
        assert_eq!(server.installation_url().await, Err(expected));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
    Ok(())
}
